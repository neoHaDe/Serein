//! SFTP через russh-sftp: просмотр, правка (атомарно), рекурсивные передачи (порт sftp.ts).

use crate::ssh::ClientHandler;
use russh::client;
use russh_sftp::client::{error::Error as SftpError, Config as SftpConfig, RawSftpSession, SftpSession};
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CANCELLED: &str = "Передача отменена";

fn gone(alive: Option<&AtomicBool>, xfer: Option<&AtomicBool>) -> Result<(), String> {
    if xfer.is_some_and(|a| !a.load(Ordering::Relaxed)) {
        Err(CANCELLED.into())
    } else if alive.is_some_and(|a| !a.load(Ordering::Relaxed)) {
        Err("Сессия закрыта".into())
    } else {
        Ok(())
    }
}

#[derive(Default)]
pub struct TransferHub {
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    by_session: Mutex<HashMap<String, Vec<String>>>,
}

impl TransferHub {
    pub fn start(&self, id: &str, session_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(true));
        self.running.lock().unwrap().insert(id.to_string(), flag.clone());
        self.by_session
            .lock()
            .unwrap()
            .entry(session_id.to_string())
            .or_default()
            .push(id.to_string());
        flag
    }

    pub fn cancel(&self, id: &str) -> bool {
        match self.running.lock().unwrap().get(id) {
            Some(f) => {
                f.store(false, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn finish(&self, id: &str) {
        self.running.lock().unwrap().remove(id);
    }

    pub fn cancel_session(&self, session_id: &str) {
        let ids = self
            .by_session
            .lock()
            .unwrap()
            .remove(session_id)
            .unwrap_or_default();
        let running = self.running.lock().unwrap();
        for id in ids {
            if let Some(f) = running.get(&id) {
                f.store(false, Ordering::Relaxed);
            }
        }
    }
}

const MAX_EDIT_SIZE: u64 = 5 * 1024 * 1024;
/// Совпадает с `ssh_client_config().maximum_packet_size`. DATA больше — зависание окна.
const SFTP_CHUNK: u32 = 32 * 1024 - 64;
const READ_INFLIGHT: usize = 16;
const PIPELINE_AFTER: u64 = 256 * 1024;
const SFTP_TIMEOUT_SECS: u64 = 300;

fn sftp_config() -> SftpConfig {
    SftpConfig {
        max_packet_len: 32 * 1024,
        max_concurrent_writes: 16,
        request_timeout_secs: SFTP_TIMEOUT_SECS,
    }
}

async fn open_stream(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
) -> Result<russh::ChannelStream<russh::client::Msg>, String> {
    let channel = {
        let h = handle.lock().await;
        h.channel_open_session().await.map_err(|e| e.to_string())?
    };
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| e.to_string())?;
    Ok(channel.into_stream())
}

/// Открывает новый SFTP-канал поверх SSH-соединения.
pub async fn open(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>) -> Result<SftpSession, String> {
    let stream = open_stream(handle).await?;
    let sftp = SftpSession::new_with_config(stream, sftp_config())
        .await
        .map_err(|e| e.to_string())?;
    sftp.set_timeout(SFTP_TIMEOUT_SECS);
    Ok(sftp)
}

async fn open_raw(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>) -> Result<RawSftpSession, String> {
    let stream = open_stream(handle).await?;
    let raw = RawSftpSession::new_with_config(stream, sftp_config());
    raw.init().await.map_err(|e| e.to_string())?;
    raw.set_timeout(SFTP_TIMEOUT_SECS);
    Ok(raw)
}

fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub async fn list(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str) -> Result<Value, String> {
    let sftp = open(handle).await?;
    let target = if path.is_empty() { ".".to_string() } else { path.to_string() };
    let abs = sftp.canonicalize(target).await.map_err(|e| e.to_string())?;
    let mut entries: Vec<Value> = Vec::new();
    let rd = sftp.read_dir(&abs).await.map_err(|e| e.to_string())?;
    for entry in rd {
        let ft = entry.file_type();
        let kind = if ft.is_dir() {
            "dir"
        } else if ft.is_symlink() {
            "link"
        } else {
            "file"
        };
        let meta = entry.metadata();
        entries.push(json!({
            "name": entry.file_name(),
            "type": kind,
            "size": meta.size.unwrap_or(0),
            "mtime": meta.mtime.unwrap_or(0) as u64 * 1000,
            "mode": meta.permissions.unwrap_or(0),
        }));
    }
    entries.sort_by(|a, b| {
        let ad = a["type"] == json!("dir");
        let bd = b["type"] == json!("dir");
        bd.cmp(&ad).then_with(|| {
            a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or(""))
        })
    });
    Ok(json!({ "path": abs, "entries": entries }))
}

/// Скачивает один удалённый файл в локальный путь (без событий) — для внешнего редактора.
pub async fn download_file(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, remote: &str, local: &str) -> Result<(), String> {
    let sftp = open(handle).await?;
    copy_remote_to_local_inner(handle, &sftp, None, "", "", remote, local, "", 0, None, None).await?;
    Ok(())
}

/// Заливает один локальный файл на удалённый путь (без событий).
pub async fn put_file(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, local: &str, remote: &str) -> Result<(), String> {
    let sftp = open(handle).await?;
    let size = tokio::fs::metadata(local).await.map(|m| m.len()).unwrap_or(0);
    copy_local_to_remote_inner(&sftp, None, "", "", local, remote, "", size, None, None).await?;
    Ok(())
}

pub async fn mkdir(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str) -> Result<(), String> {
    let sftp = open(handle).await?;
    sftp.create_dir(path).await.map_err(|e| e.to_string())
}

pub async fn remove(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str, is_dir: bool) -> Result<(), String> {
    let sftp = open(handle).await?;
    if is_dir {
        sftp.remove_dir(path).await.map_err(|e| e.to_string())
    } else {
        sftp.remove_file(path).await.map_err(|e| e.to_string())
    }
}

pub async fn rename(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, from: &str, to: &str) -> Result<(), String> {
    let sftp = open(handle).await?;
    sftp.rename(from, to).await.map_err(|e| e.to_string())
}

pub async fn read_file(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, remote: &str) -> Result<Value, String> {
    let sftp = open(handle).await?;
    let meta = sftp.metadata(remote).await.map_err(|e| e.to_string())?;
    let size = meta.size.unwrap_or(0);
    let mode = meta.permissions.unwrap_or(0o644);
    let mtime = meta.mtime.unwrap_or(0) as u64 * 1000;
    if size > MAX_EDIT_SIZE {
        return Ok(json!({ "content": "", "eol": "lf", "mode": mode, "mtime": mtime, "tooLarge": true }));
    }
    let mut file = sftp.open(remote).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
    file.shutdown().await.ok();
    if buf.iter().take(8192).any(|b| *b == 0) {
        return Ok(json!({ "content": "", "eol": "lf", "mode": mode, "mtime": mtime, "binary": true }));
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let eol = if text.contains("\r\n") { "crlf" } else { "lf" };
    let content = if eol == "crlf" { text.replace("\r\n", "\n") } else { text };
    Ok(json!({ "content": content, "eol": eol, "mode": mode, "mtime": mtime }))
}

pub async fn write_file(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &str,
    content: &str,
    mode: u32,
    base_mtime: u64,
    eol: &str,
) -> Result<Value, String> {
    let sftp = open(handle).await?;
    // Детект конфликта по mtime.
    if let Ok(meta) = sftp.metadata(remote).await {
        let cur = meta.mtime.unwrap_or(0) as u64 * 1000;
        if cur != 0 && base_mtime != 0 && cur.abs_diff(base_mtime) > 1000 {
            return Ok(json!({ "ok": false, "conflict": true }));
        }
    }
    let data = if eol == "crlf" { content.replace('\n', "\r\n") } else { content.to_string() };
    let dir = Path::new(remote).parent().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    let base = Path::new(remote).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = format!("{dir}/.{base}.terminal-tmp");

    {
        let mut f = sftp.create(&tmp).await.map_err(|e| e.to_string())?;
        f.write_all(data.as_bytes()).await.map_err(|e| e.to_string())?;
        f.flush().await.map_err(|e| e.to_string())?;
        f.shutdown().await.ok();
    }
    let _ = mode; // права: russh-sftp выставит дефолтные; точную установку добавим позже
    // rename поверх (с фолбэком через unlink).
    if sftp.rename(&tmp, remote).await.is_err() {
        let _ = sftp.remove_file(remote).await;
        sftp.rename(&tmp, remote).await.map_err(|e| {
            let _ = tmp;
            e.to_string()
        })?;
    }
    let new_mtime = sftp
        .metadata(remote)
        .await
        .ok()
        .and_then(|m| m.mtime)
        .map(|t| t as u64 * 1000)
        .unwrap_or(base_mtime);
    Ok(json!({ "ok": true, "mtime": new_mtime }))
}

/// Скачивает один файл с прогрессом (эмит `sftp-transfer`).
async fn copy_remote_to_local(
    app: Option<&AppHandle>,
    ssh: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    sftp: &SftpSession,
    item_id: &str,
    session_id: &str,
    remote: &str,
    local: &str,
    rel: &str,
    size: u64,
    alive: Option<&AtomicBool>,
    xfer: Option<&AtomicBool>,
) -> Result<u64, String> {
    copy_remote_to_local_inner(ssh, sftp, app, item_id, session_id, remote, local, rel, size, alive, xfer).await
}

async fn copy_remote_to_local_inner(
    ssh: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    sftp: &SftpSession,
    app: Option<&AppHandle>,
    item_id: &str,
    session_id: &str,
    remote: &str,
    local: &str,
    rel: &str,
    mut size: u64,
    alive: Option<&AtomicBool>,
    xfer: Option<&AtomicBool>,
) -> Result<u64, String> {
    gone(alive, xfer)?;
    if size == 0 {
        size = sftp.metadata(remote).await.ok().and_then(|m| m.size).unwrap_or(0);
    }
    if size >= PIPELINE_AFTER {
        pipelined_download(ssh, app, item_id, session_id, remote, local, rel, size, alive, xfer).await
    } else {
        sequential_download(sftp, app, item_id, session_id, remote, local, rel, size, alive, xfer).await
    }
}

async fn sequential_download(
    sftp: &SftpSession,
    app: Option<&AppHandle>,
    item_id: &str,
    session_id: &str,
    remote: &str,
    local: &str,
    rel: &str,
    size: u64,
    alive: Option<&AtomicBool>,
    xfer: Option<&AtomicBool>,
) -> Result<u64, String> {
    gone(alive, xfer)?;
    if let Some(parent) = Path::new(local).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let mut rf = sftp.open(remote).await.map_err(|e| e.to_string())?;
    let mut lf = tokio::fs::File::create(local).await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; SFTP_CHUNK as usize];
    let mut transferred: u64 = 0;
    let mut last_emit: u64 = 0;
    loop {
        gone(alive, xfer)?;
        let n = rf.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        lf.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
        transferred += n as u64;
        if let Some(app) = app {
            if transferred - last_emit >= 262144 {
                last_emit = transferred;
                emit_transfer(app, item_id, session_id, "download", local, remote, rel, size, transferred, "active", None);
            }
        }
    }
    lf.flush().await.ok();
    rf.shutdown().await.ok();
    Ok(transferred)
}

/// Несколько SSH_FXP_READ в полёте — иначе download упирается в RTT и на 1 ГБ «замирает».
async fn pipelined_download(
    ssh: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    app: Option<&AppHandle>,
    item_id: &str,
    session_id: &str,
    remote: &str,
    local: &str,
    rel: &str,
    size: u64,
    alive: Option<&AtomicBool>,
    xfer: Option<&AtomicBool>,
) -> Result<u64, String> {
    if let Some(parent) = Path::new(local).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let raw = open_raw(ssh).await?;
    let opened = raw
        .open(remote.to_string(), OpenFlags::READ, FileAttributes::empty())
        .await
        .map_err(|e| e.to_string())?;
    let fh = opened.handle;
    let mut lf = tokio::fs::File::create(local).await.map_err(|e| e.to_string())?;
    let mut offset = 0u64;
    let mut transferred = 0u64;
    let mut last_emit = 0u64;
    let mut eof = false;
    while !eof && (size == 0 || offset < size) {
        gone(alive, xfer)?;
        let mut batch = Vec::new();
        let mut offs = Vec::new();
        for _ in 0..READ_INFLIGHT {
            if size > 0 && offset >= size {
                break;
            }
            let len = if size > 0 {
                (size - offset).min(SFTP_CHUNK as u64) as u32
            } else {
                SFTP_CHUNK
            };
            if len == 0 {
                break;
            }
            offs.push(offset);
            batch.push(raw.read(fh.clone(), offset, len));
            offset += u64::from(len);
        }
        if batch.is_empty() {
            break;
        }
        let results = futures::future::join_all(batch).await;
        for res in results {
            match res {
                Ok(pkt) => {
                    if pkt.data.is_empty() {
                        eof = true;
                        break;
                    }
                    lf.write_all(&pkt.data).await.map_err(|e| e.to_string())?;
                    transferred += pkt.data.len() as u64;
                    if let Some(app) = app {
                        if transferred - last_emit >= 262144 {
                            last_emit = transferred;
                            emit_transfer(app, item_id, session_id, "download", local, remote, rel, size, transferred, "active", None);
                        }
                    }
                }
                Err(SftpError::Status(st)) if st.status_code == StatusCode::Eof => {
                    eof = true;
                    break;
                }
                Err(e) => {
                    let _ = raw.close(fh.clone()).await;
                    return Err(e.to_string());
                }
            }
        }
    }
    lf.flush().await.ok();
    let _ = raw.close(fh).await;
    let _ = raw.close_session();
    Ok(transferred)
}

async fn copy_local_to_remote(
    app: Option<&AppHandle>,
    sftp: &SftpSession,
    item_id: &str,
    session_id: &str,
    local: &str,
    remote: &str,
    rel: &str,
    size: u64,
    alive: Option<&AtomicBool>,
    xfer: Option<&AtomicBool>,
) -> Result<u64, String> {
    copy_local_to_remote_inner(sftp, app, item_id, session_id, local, remote, rel, size, alive, xfer).await
}

async fn copy_local_to_remote_inner(
    sftp: &SftpSession,
    app: Option<&AppHandle>,
    item_id: &str,
    session_id: &str,
    local: &str,
    remote: &str,
    rel: &str,
    size: u64,
    alive: Option<&AtomicBool>,
    xfer: Option<&AtomicBool>,
) -> Result<u64, String> {
    let mut lf = tokio::fs::File::open(local).await.map_err(|e| e.to_string())?;
    let mut rf = sftp.create(remote).await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; SFTP_CHUNK as usize];
    let mut transferred: u64 = 0;
    let mut last_emit: u64 = 0;
    loop {
        gone(alive, xfer)?;
        let n = lf.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        rf.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
        transferred += n as u64;
        if let Some(app) = app {
            if transferred - last_emit >= 262144 {
                last_emit = transferred;
                emit_transfer(app, item_id, session_id, "upload", local, remote, rel, size, transferred, "active", None);
            }
        }
    }
    rf.flush().await.ok();
    rf.shutdown().await.ok();
    Ok(transferred)
}

/// Для бенча: те же пути, что у UI.
pub async fn copy_file_up(sftp: &SftpSession, local: &str, remote: &str) -> Result<u64, String> {
    let size = tokio::fs::metadata(local).await.map(|m| m.len()).unwrap_or(0);
    copy_local_to_remote_inner(sftp, None, "", "", local, remote, "", size, None, None).await
}

pub async fn copy_file_down(
    ssh: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    sftp: &SftpSession,
    remote: &str,
    local: &str,
) -> Result<u64, String> {
    copy_remote_to_local_inner(ssh, sftp, None, "", "", remote, local, "", 0, None, None).await
}

#[allow(clippy::too_many_arguments)]
fn emit_transfer(
    app: &AppHandle,
    id: &str,
    session_id: &str,
    direction: &str,
    local: &str,
    remote: &str,
    filename: &str,
    size: u64,
    transferred: u64,
    state: &str,
    error: Option<&str>,
) {
    let _ = app.emit(
        "sftp-transfer",
        json!({
            "id": id, "sessionId": session_id, "direction": direction,
            "localPath": local, "remotePath": remote, "filename": filename,
            "size": size, "transferred": transferred, "state": state, "error": error,
        }),
    );
}

/// Рекурсивно заливает локальный путь (файл/папка) в remoteDir, эмитя события.
pub async fn upload_path(
    app: AppHandle,
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    session_id: &str,
    local: &str,
    remote_dir: &str,
    alive: Option<&AtomicBool>,
    hub: &TransferHub,
) -> Result<(), String> {
    gone(alive, None)?;
    let sftp = open(handle).await?;
    let local = local.replace('\\', "/");
    let root_name = Path::new(&local).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    let mut files: Vec<(String, String, String, u64)> = Vec::new();
    collect_local(&local, &join_remote(remote_dir, &root_name), &root_name, &mut files).await?;
    for (lp, rp, rel, size) in files {
        gone(alive, None)?;
        let id = uuid::Uuid::new_v4().to_string();
        let flag = hub.start(&id, session_id);
        emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "active", None);
        if let Some(parent) = Path::new(&rp).parent() {
            let _ = ensure_remote_dir(&sftp, &parent.to_string_lossy().replace('\\', "/")).await;
        }
        let result = copy_local_to_remote(Some(&app), &sftp, &id, session_id, &lp, &rp, &rel, size, alive, Some(&flag)).await;
        hub.finish(&id);
        match result {
            Ok(_) => emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, size, "done", None),
            Err(e) if e == CANCELLED => {
                emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "canceled", None)
            }
            Err(e) => emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "error", Some(&e)),
        }
    }
    Ok(())
}

/// Рекурсивно скачивает удалённый путь (файл/папка) в localDir, эмитя события.
pub async fn download_path(
    app: AppHandle,
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    session_id: &str,
    remote: &str,
    local_dir: &str,
    alive: Option<&AtomicBool>,
    hub: &TransferHub,
) -> Result<(), String> {
    gone(alive, None)?;
    let sftp = open(handle).await?;
    let root_name = Path::new(remote).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "download".into());
    let local_dir = local_dir.replace('\\', "/");
    let mut files: Vec<(String, String, String, u64)> = Vec::new();
    collect_remote(&sftp, remote, &format!("{local_dir}/{root_name}"), &root_name, &mut files).await?;
    for (lp, rp, rel, size) in files {
        gone(alive, None)?;
        let id = uuid::Uuid::new_v4().to_string();
        let flag = hub.start(&id, session_id);
        emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "active", None);
        let result = copy_remote_to_local(Some(&app), handle, &sftp, &id, session_id, &rp, &lp, &rel, size, alive, Some(&flag)).await;
        hub.finish(&id);
        match result {
            Ok(n) => emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, n, "done", None),
            Err(e) if e == CANCELLED => {
                emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "canceled", None)
            }
            Err(e) => emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "error", Some(&e)),
        }
    }
    Ok(())
}

async fn ensure_remote_dir(sftp: &SftpSession, dir: &str) -> Result<(), String> {
    let mut cur = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        cur = if cur.is_empty() && dir.starts_with('/') {
            format!("/{part}")
        } else if cur.is_empty() {
            part.to_string()
        } else {
            format!("{cur}/{part}")
        };
        let _ = sftp.create_dir(&cur).await; // существующая папка → молча
    }
    Ok(())
}

// Рекурсивный обход локальной ФС (boxed — рекурсия в async).
fn collect_local<'a>(
    local: &'a str,
    remote: &'a str,
    rel: &'a str,
    out: &'a mut Vec<(String, String, String, u64)>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
    Box::pin(async move {
        let meta = tokio::fs::metadata(local).await.map_err(|e| e.to_string())?;
        if meta.is_dir() {
            let mut rd = tokio::fs::read_dir(local).await.map_err(|e| e.to_string())?;
            while let Some(ent) = rd.next_entry().await.map_err(|e| e.to_string())? {
                let name = ent.file_name().to_string_lossy().to_string();
                let lp = format!("{local}/{name}");
                let rp = format!("{remote}/{name}");
                let r = format!("{rel}/{name}");
                collect_local(&lp, &rp, &r, out).await?;
            }
        } else {
            out.push((local.to_string(), remote.to_string(), rel.to_string(), meta.len()));
        }
        Ok(())
    })
}

fn collect_remote<'a>(
    sftp: &'a SftpSession,
    remote: &'a str,
    local: &'a str,
    rel: &'a str,
    out: &'a mut Vec<(String, String, String, u64)>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
    Box::pin(async move {
        let meta = sftp.metadata(remote).await.map_err(|e| e.to_string())?;
        if meta.file_type().is_dir() {
            let rd = sftp.read_dir(remote).await.map_err(|e| e.to_string())?;
            for entry in rd {
                let name = entry.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let rp = format!("{remote}/{name}");
                let lp = format!("{local}/{name}");
                let r = format!("{rel}/{name}");
                collect_remote(sftp, &rp, &lp, &r, out).await?;
            }
        } else {
            out.push((local.to_string(), remote.to_string(), rel.to_string(), meta.size.unwrap_or(0)));
        }
        Ok(())
    })
}
