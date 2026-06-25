//! SFTP через russh-sftp: просмотр, правка (атомарно), рекурсивные передачи (порт sftp.ts).

use crate::ssh::ClientHandler;
use russh::client;
use russh_sftp::client::SftpSession;
use serde_json::{json, Value};
use std::path::Path;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_EDIT_SIZE: u64 = 5 * 1024 * 1024;

/// Открывает новый SFTP-канал поверх SSH-соединения.
pub async fn open(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>) -> Result<SftpSession, String> {
    let channel = {
        let h = handle.lock().await;
        h.channel_open_session().await.map_err(|e| e.to_string())?
    };
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| e.to_string())?;
    SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| e.to_string())
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
    if let Some(p) = Path::new(local).parent() {
        let _ = tokio::fs::create_dir_all(p).await;
    }
    let mut rf = sftp.open(remote).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    rf.read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
    tokio::fs::write(local, buf).await.map_err(|e| e.to_string())
}

/// Заливает один локальный файл на удалённый путь (без событий).
pub async fn put_file(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, local: &str, remote: &str) -> Result<(), String> {
    let sftp = open(handle).await?;
    let data = tokio::fs::read(local).await.map_err(|e| e.to_string())?;
    let mut f = sftp.create(remote).await.map_err(|e| e.to_string())?;
    f.write_all(&data).await.map_err(|e| e.to_string())?;
    f.flush().await.ok();
    f.shutdown().await.ok();
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
    app: &AppHandle,
    sftp: &SftpSession,
    item_id: &str,
    session_id: &str,
    remote: &str,
    local: &str,
    rel: &str,
    size: u64,
) -> Result<(), String> {
    if let Some(parent) = Path::new(local).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let mut rf = sftp.open(remote).await.map_err(|e| e.to_string())?;
    let mut lf = tokio::fs::File::create(local).await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 65536];
    let mut transferred: u64 = 0;
    let mut last_emit: u64 = 0;
    loop {
        let n = rf.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        lf.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
        transferred += n as u64;
        if transferred - last_emit >= 262144 {
            last_emit = transferred;
            emit_transfer(app, item_id, session_id, "download", local, remote, rel, size, transferred, "active", None);
        }
    }
    lf.flush().await.ok();
    Ok(())
}

async fn copy_local_to_remote(
    app: &AppHandle,
    sftp: &SftpSession,
    item_id: &str,
    session_id: &str,
    local: &str,
    remote: &str,
    rel: &str,
    size: u64,
) -> Result<(), String> {
    let mut lf = tokio::fs::File::open(local).await.map_err(|e| e.to_string())?;
    let mut rf = sftp.create(remote).await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 65536];
    let mut transferred: u64 = 0;
    let mut last_emit: u64 = 0;
    loop {
        let n = lf.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        rf.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
        transferred += n as u64;
        if transferred - last_emit >= 262144 {
            last_emit = transferred;
            emit_transfer(app, item_id, session_id, "upload", local, remote, rel, size, transferred, "active", None);
        }
    }
    rf.flush().await.ok();
    rf.shutdown().await.ok();
    Ok(())
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
pub async fn upload_path(app: AppHandle, handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, session_id: &str, local: &str, remote_dir: &str) -> Result<(), String> {
    let sftp = open(handle).await?;
    let local = local.replace('\\', "/");
    let root_name = Path::new(&local).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    let mut files: Vec<(String, String, String, u64)> = Vec::new(); // (local, remote, rel, size)
    collect_local(&local, &join_remote(remote_dir, &root_name), &root_name, &mut files).await?;
    for (lp, rp, rel, size) in files {
        let id = uuid::Uuid::new_v4().to_string();
        emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "active", None);
        if let Some(parent) = Path::new(&rp).parent() {
            let _ = ensure_remote_dir(&sftp, &parent.to_string_lossy().replace('\\', "/")).await;
        }
        match copy_local_to_remote(&app, &sftp, &id, session_id, &lp, &rp, &rel, size).await {
            Ok(_) => emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, size, "done", None),
            Err(e) => emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "error", Some(&e)),
        }
    }
    Ok(())
}

/// Рекурсивно скачивает удалённый путь (файл/папка) в localDir, эмитя события.
pub async fn download_path(app: AppHandle, handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, session_id: &str, remote: &str, local_dir: &str) -> Result<(), String> {
    let sftp = open(handle).await?;
    let root_name = Path::new(remote).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "download".into());
    let local_dir = local_dir.replace('\\', "/");
    let mut files: Vec<(String, String, String, u64)> = Vec::new();
    collect_remote(&sftp, remote, &format!("{local_dir}/{root_name}"), &root_name, &mut files).await?;
    for (lp, rp, rel, size) in files {
        let id = uuid::Uuid::new_v4().to_string();
        emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "active", None);
        match copy_remote_to_local(&app, &sftp, &id, session_id, &rp, &lp, &rel, size).await {
            Ok(_) => emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, size, "done", None),
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
