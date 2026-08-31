//! SFTP через russh-sftp: просмотр, правка (атомарно), рекурсивные передачи (порт sftp.ts).

use crate::ssh::{ClientHandler, SharedHandle};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures::stream::{self, FuturesUnordered, StreamExt};
use russh::client;
use russh_sftp::client::rawsession::Limits;
use russh_sftp::client::{error::Error as SftpError, Config as SftpConfig, RawSftpSession, SftpSession};
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::sync::Notify;

pub(crate) const CANCELLED: &str = "Передача отменена";
pub(crate) const PAUSED: &str = "paused";
/// Параллельных файлов по умолчанию (роадмап 3.2).
const TRANSFER_SLOTS: usize = 4;
/// Верхняя граница пула — не открывать безлимит каналов.
const TRANSFER_MAX: usize = 8;

pub struct XferCtrl {
    live: AtomicBool,
    paused: AtomicBool,
    wake: Notify,
    key: String,
}

impl XferCtrl {
    fn new(key: String) -> Arc<Self> {
        Arc::new(Self {
            live: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            wake: Notify::new(),
            key,
        })
    }

    pub(crate) fn is_live(&self) -> bool {
        self.live.load(Ordering::Relaxed)
    }

    fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    fn cancel(&self) {
        self.live.store(false, Ordering::Relaxed);
        self.wake.notify_waiters();
    }

    fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
        self.wake.notify_waiters();
    }

    fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
        self.wake.notify_waiters();
    }
}

#[derive(Clone)]
struct SlotPool {
    active: Arc<Mutex<usize>>,
    limit: Arc<AtomicUsize>,
    wait: Arc<Notify>,
}

struct SlotGuard {
    pool: SlotPool,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        if let Ok(mut n) = self.pool.active.lock() {
            *n = n.saturating_sub(1);
        }
        self.pool.wait.notify_one();
    }
}

impl SlotPool {
    fn new(limit: usize) -> Self {
        Self {
            active: Arc::new(Mutex::new(0)),
            limit: Arc::new(AtomicUsize::new(limit.clamp(1, TRANSFER_MAX))),
            wait: Arc::new(Notify::new()),
        }
    }

    fn set_limit(&self, n: usize) {
        self.limit.store(n.clamp(1, TRANSFER_MAX), Ordering::Relaxed);
        self.wait.notify_waiters();
    }

    async fn acquire(&self) -> SlotGuard {
        loop {
            {
                let mut n = crate::sync::lock(&self.active);
                let cap = self.limit.load(Ordering::Relaxed).clamp(1, TRANSFER_MAX);
                if *n < cap {
                    *n += 1;
                    return SlotGuard { pool: self.clone() };
                }
            }
            self.wait.notified().await;
        }
    }
}

fn gone(alive: Option<&AtomicBool>, xfer: Option<&XferCtrl>) -> Result<(), String> {
    if xfer.is_some_and(|c| !c.is_live()) {
        Err(CANCELLED.into())
    } else if alive.is_some_and(|a| !a.load(Ordering::Relaxed)) {
        Err("Сессия закрыта".into())
    } else {
        Ok(())
    }
}

async fn wait_cancel(alive: Option<&AtomicBool>, xfer: Option<&XferCtrl>) {
    if alive.is_none() && xfer.is_none() {
        std::future::pending::<()>().await;
        return;
    }
    loop {
        if gone(alive, xfer).is_err() {
            return;
        }
        if let Some(c) = xfer {
            tokio::select! {
                _ = c.wake.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
            }
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn wait_if_paused(
    app: Option<&AppHandle>,
    alive: Option<&AtomicBool>,
    xfer: Option<&XferCtrl>,
    item_id: &str,
    session_id: &str,
    direction: &str,
    local: &str,
    remote: &str,
    rel: &str,
    size: u64,
    transferred: u64,
) -> Result<(), String> {
    let Some(c) = xfer else {
        return Ok(());
    };
    if !c.is_paused() {
        return Ok(());
    }
    if let Some(app) = app {
        emit_transfer(app, item_id, session_id, direction, local, remote, rel, size, transferred, PAUSED, None);
    }
    while c.is_paused() {
        gone(alive, xfer)?;
        tokio::select! {
            _ = c.wake.notified() => {}
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
        }
    }
    gone(alive, xfer)?;
    if let Some(app) = app {
        emit_transfer(app, item_id, session_id, direction, local, remote, rel, size, transferred, "active", None);
    }
    Ok(())
}

pub(crate) fn dup_key(session_id: &str, direction: &str, local: &str, remote: &str) -> String {
    format!(
        "{session_id}|{direction}|{}|{remote}",
        local.replace('\\', "/")
    )
}

pub struct TransferHub {
    running: Arc<Mutex<HashMap<String, Arc<XferCtrl>>>>,
    by_session: Arc<Mutex<HashMap<String, Vec<String>>>>,
    dup: Arc<Mutex<HashSet<String>>>,
    slots: SlotPool,
}

impl Clone for TransferHub {
    fn clone(&self) -> Self {
        Self {
            running: self.running.clone(),
            by_session: self.by_session.clone(),
            dup: self.dup.clone(),
            slots: self.slots.clone(),
        }
    }
}

fn concurrency_from_settings() -> usize {
    crate::store::settings_get()
        .get("sftpConcurrency")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(TRANSFER_SLOTS)
        .clamp(1, TRANSFER_MAX)
}

impl Default for TransferHub {
    fn default() -> Self {
        Self {
            running: Arc::new(Mutex::new(HashMap::new())),
            by_session: Arc::new(Mutex::new(HashMap::new())),
            dup: Arc::new(Mutex::new(HashSet::new())),
            slots: SlotPool::new(concurrency_from_settings()),
        }
    }
}

impl TransferHub {
    /// None — такой файл уже в очереди или качается.
    pub fn start(&self, id: &str, session_id: &str, key: String) -> Option<Arc<XferCtrl>> {
        {
            let mut dup = crate::sync::lock(&self.dup);
            if !dup.insert(key.clone()) {
                return None;
            }
        }
        let ctrl = XferCtrl::new(key);
        crate::sync::lock(&self.running).insert(id.to_string(), ctrl.clone());
        crate::sync::lock(&self.by_session)
            .entry(session_id.to_string())
            .or_default()
            .push(id.to_string());
        Some(ctrl)
    }

    pub fn cancel(&self, id: &str) -> bool {
        match crate::sync::lock(&self.running).get(id) {
            Some(c) => {
                c.cancel();
                true
            }
            None => false,
        }
    }

    pub fn pause(&self, id: &str) -> bool {
        match crate::sync::lock(&self.running).get(id) {
            Some(c) if c.is_live() => {
                c.pause();
                true
            }
            _ => false,
        }
    }

    pub fn resume(&self, id: &str) -> bool {
        match crate::sync::lock(&self.running).get(id) {
            Some(c) if c.is_live() => {
                c.resume();
                true
            }
            _ => false,
        }
    }

    pub fn finish(&self, id: &str) {
        if let Some(c) = crate::sync::lock(&self.running).remove(id) {
            crate::sync::lock(&self.dup).remove(&c.key);
        }
    }

    pub fn set_limit(&self, n: usize) {
        self.slots.set_limit(n);
    }

    pub fn cancel_session(&self, session_id: &str) {
        let ids = crate::sync::lock(&self.by_session)
            .remove(session_id)
            .unwrap_or_default();
        let running = crate::sync::lock(&self.running);
        for id in ids {
            if let Some(c) = running.get(&id) {
                c.cancel();
            }
        }
    }
}

const MAX_EDIT_SIZE: u64 = 5 * 1024 * 1024;
/// WRITE укладываем в SSH-пакет 32 КиБ. READ может быть крупнее: сервер режет CHANNEL_DATA сам.
const SFTP_CHUNK: u32 = 32 * 1024 - 64;
/// Потолок SSH_FXP_READ, если сервер отдал limits@openssh.com.
const SFTP_READ_MAX: u32 = 256 * 1024 - 64;
const READ_INFLIGHT: usize = 64;
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

/// Проверка доступности SFTP-подсистемы на соединении.
pub async fn probe(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>) -> bool {
    open(handle).await.is_ok()
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

pub(crate) fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Проверка удалённого пути перед операцией.
///
/// `list` разрешает путь через `canonicalize` на сервере, а остальные операции работают
/// с тем, что пришло из UI, — поэтому здесь требуем абсолютный путь без `..` и управляющих
/// символов. Иначе одна кривая строка в поле пути уводит удаление или заливку туда,
/// куда пользователь не смотрел.
pub fn check_remote_path(p: &str) -> Result<(), String> {
    if p.trim().is_empty() {
        return Err("Пустой удалённый путь".into());
    }
    if !p.starts_with('/') {
        return Err(format!("Ожидался абсолютный удалённый путь, получен «{p}»"));
    }
    if p.contains('\0') || p.contains('\n') || p.contains('\r') {
        return Err("Недопустимый символ в удалённом пути".into());
    }
    if p.split('/').any(|seg| seg == "..") {
        return Err(format!("Удалённый путь с «..» недопустим: «{p}»"));
    }
    Ok(())
}

pub async fn list(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str) -> Result<Value, String> {
    let sftp = open(handle).await?;
    let target = if path.is_empty() { ".".to_string() } else { path.to_string() };
    let abs = sftp.canonicalize(target).await.map_err(|e| e.to_string())?;
    let mut entries: Vec<Value> = Vec::new();
    let rd = sftp.read_dir(&abs).await.map_err(|e| e.to_string())?;
    for entry in rd {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let ft = entry.file_type();
        let kind = if ft.is_dir() {
            "dir"
        } else if ft.is_symlink() {
            "link"
        } else {
            "file"
        };
        let meta = entry.metadata();
        let full = join_remote(&abs, &name);
        let (target, link_type) = if kind == "link" {
            let t = sftp.read_link(&full).await.ok();
            let lt = match sftp.metadata(&full).await {
                Ok(m) if m.file_type().is_dir() => Some("dir"),
                Ok(_) => Some("file"),
                Err(_) => Some("broken"),
            };
            (t, lt)
        } else {
            (None, None)
        };
        entries.push(json!({
            "name": name,
            "type": kind,
            "size": meta.size.unwrap_or(0),
            "mtime": meta.mtime.unwrap_or(0) as u64 * 1000,
            "mode": meta.permissions.unwrap_or(0),
            "target": target,
            "linkType": link_type,
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
    check_remote_path(remote)?;
    let sftp = open(handle).await?;
    copy_remote_to_local_inner(handle, &sftp, None, "", "", remote, local, "", 0, None, None).await?;
    Ok(())
}

/// Заливает один локальный файл на удалённый путь (без событий).
pub async fn put_file(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, local: &str, remote: &str) -> Result<(), String> {
    check_remote_path(remote)?;
    let sftp = open(handle).await?;
    let size = tokio::fs::metadata(local).await.map(|m| m.len()).unwrap_or(0);
    copy_local_to_remote_inner(&sftp, None, "", "", local, remote, "", size, None, None).await?;
    Ok(())
}

pub async fn mkdir(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str) -> Result<(), String> {
    check_remote_path(path)?;
    let sftp = open(handle).await?;
    sftp.create_dir(path).await.map_err(|e| e.to_string())
}

/// Насколько глубоко готовы спускаться при удалении каталога.
///
/// Не «побольше на всякий случай»: ограничение защищает от петли из симлинков и от
/// подсунутого дерева нелепой глубины. Тридцать уровней — заметно больше, чем встречается
/// в реальных каталогах, и заведомо меньше, чем нужно, чтобы уйти в бесконечность.
const MAX_REMOVE_DEPTH: usize = 30;

pub async fn remove(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str, is_dir: bool) -> Result<(), String> {
    check_remote_path(path)?;
    let sftp = open(handle).await?;
    if is_dir {
        remove_dir_all(&sftp, path).await
    } else {
        sftp.remove_file(path).await.map_err(|e| e.to_string())
    }
}

/// Удаляет каталог вместе с содержимым.
///
/// Раньше здесь был голый `remove_dir`, который в SFTP работает только на пустом каталоге.
/// Любая попытка удалить папку с файлами из файлового менеджера заканчивалась протокольным
/// «Failure: Failure» — сообщением, из которого пользователю не следует ровно ничего.
///
/// Симлинки удаляем как файлы и внутрь НЕ заходим: ссылка на `/etc` не повод снести `/etc`.
async fn remove_dir_all(sftp: &SftpSession, root: &str) -> Result<(), String> {
    // Обход без рекурсии: async-рекурсия требует боксинга на каждом уровне, а здесь
    // достаточно собрать каталоги сверху вниз и удалить их снизу вверх.
    let mut to_visit = vec![(root.to_string(), 0usize)];
    let mut dirs: Vec<String> = Vec::new();

    while let Some((dir, depth)) = to_visit.pop() {
        if depth > MAX_REMOVE_DEPTH {
            return Err(format!(
                "Слишком глубокая вложенность в «{dir}» (больше {MAX_REMOVE_DEPTH} уровней) — удаление остановлено"
            ));
        }
        let entries = sftp
            .read_dir(&dir)
            .await
            .map_err(|e| format!("Не удалось прочитать «{dir}»: {e}"))?;
        for entry in entries {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let full = join_remote(&dir, &name);
            let ft = entry.file_type();
            if ft.is_dir() && !ft.is_symlink() {
                to_visit.push((full, depth + 1));
            } else {
                sftp.remove_file(&full)
                    .await
                    .map_err(|e| format!("Не удалось удалить «{full}»: {e}"))?;
            }
        }
        dirs.push(dir);
    }

    // Снизу вверх: SFTP умеет удалять только пустые каталоги.
    for dir in dirs.into_iter().rev() {
        sftp.remove_dir(&dir)
            .await
            .map_err(|e| format!("Не удалось удалить каталог «{dir}»: {e}"))?;
    }
    Ok(())
}

pub async fn rename(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, from: &str, to: &str) -> Result<(), String> {
    check_remote_path(from)?;
    check_remote_path(to)?;
    let sftp = open(handle).await?;
    sftp.rename(from, to).await.map_err(|e| e.to_string())
}

pub async fn chmod(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str, mode: u32) -> Result<(), String> {
    check_remote_path(path)?;
    let sftp = open(handle).await?;
    let meta = sftp.symlink_metadata(path).await.map_err(|e| e.to_string())?;
    let old = meta.permissions.unwrap_or(0);
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some((old & !0o777) | (mode & 0o777));
    sftp.set_metadata(path, attrs).await.map_err(|e| e.to_string())
}

const MAX_PREVIEW: u64 = 8 * 1024 * 1024;

pub async fn preview(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, remote: &str) -> Result<Value, String> {
    check_remote_path(remote)?;
    let sftp = open(handle).await?;
    let meta = sftp.metadata(remote).await.map_err(|e| e.to_string())?;
    let size = meta.size.unwrap_or(0);
    if size > MAX_PREVIEW {
        return Ok(json!({ "kind": "tooLarge", "size": size }));
    }
    let mut file = sftp.open(remote).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
    file.shutdown().await.ok();
    Ok(json!({
        "kind": "bytes",
        "size": size,
        "base64": STANDARD.encode(&buf)
    }))
}

pub async fn read_file(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, remote: &str) -> Result<Value, String> {
    check_remote_path(remote)?;
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
    check_remote_path(remote)?;
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
    xfer: Option<&XferCtrl>,
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
    xfer: Option<&XferCtrl>,
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
    xfer: Option<&XferCtrl>,
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
        wait_if_paused(app, alive, xfer, item_id, session_id, "download", local, remote, rel, size, transferred).await?;
        let n = tokio::select! {
            _ = wait_cancel(alive, xfer) => return Err(CANCELLED.into()),
            n = rf.read(&mut buf) => n.map_err(|e| e.to_string())?,
        };
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
    xfer: Option<&XferCtrl>,
) -> Result<u64, String> {
    if let Some(parent) = Path::new(local).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let mut raw = open_raw(ssh).await?;
    let mut chunk = SFTP_CHUNK;
    if let Ok(ext) = raw.limits().await {
        if ext.max_read_len > 1024 {
            chunk = ext.max_read_len.min(u64::from(SFTP_READ_MAX)) as u32;
        }
        raw.set_limits(Limits::from(ext));
    }
    let raw = Arc::new(raw);
    let opened = raw
        .open(remote.to_string(), OpenFlags::READ, FileAttributes::empty())
        .await
        .map_err(|e| e.to_string())?;
    let fh = opened.handle;
    let mut lf = BufWriter::with_capacity(
        1024 * 1024,
        tokio::fs::File::create(local).await.map_err(|e| e.to_string())?,
    );
    let mut next_send = 0u64;
    let mut next_write = 0u64;
    let mut transferred = 0u64;
    let mut last_emit = 0u64;
    let mut eof = false;
    let mut inflight = FuturesUnordered::new();
    let mut ready: BTreeMap<u64, Vec<u8>> = BTreeMap::new();

    loop {
        gone(alive, xfer)?;
        wait_if_paused(app, alive, xfer, item_id, session_id, "download", local, remote, rel, size, transferred).await?;
        while !eof
            && !xfer.is_some_and(|c| c.is_paused())
            && inflight.len() < READ_INFLIGHT
            && (size == 0 || next_send < size)
        {
            let len = if size > 0 {
                (size - next_send).min(u64::from(chunk)) as u32
            } else {
                chunk
            };
            if len == 0 {
                break;
            }
            let off = next_send;
            next_send += u64::from(len);
            let raw = raw.clone();
            let fh = fh.clone();
            inflight.push(async move { (off, raw.read(fh, off, len).await) });
        }
        let next = tokio::select! {
            _ = wait_cancel(alive, xfer) => {
                let _ = raw.close_session();
                return Err(CANCELLED.into());
            }
            next = inflight.next() => next,
        };
        let Some((off, res)) = next else {
            break;
        };
        match res {
            Ok(pkt) => {
                if pkt.data.is_empty() {
                    eof = true;
                    continue;
                }
                ready.insert(off, pkt.data);
                while let Some(data) = ready.remove(&next_write) {
                    let n = data.len() as u64;
                    lf.write_all(&data).await.map_err(|e| e.to_string())?;
                    next_write += n;
                    transferred += n;
                    if let Some(app) = app {
                        if transferred - last_emit >= 1024 * 1024 {
                            last_emit = transferred;
                            emit_transfer(
                                app, item_id, session_id, "download", local, remote, rel, size,
                                transferred, "active", None,
                            );
                        }
                    }
                }
            }
            Err(SftpError::Status(st)) if st.status_code == StatusCode::Eof => {
                eof = true;
            }
            Err(e) => {
                let _ = raw.close_session();
                return Err(e.to_string());
            }
        }
        if eof && inflight.is_empty() {
            break;
        }
        if size > 0 && next_write >= size && inflight.is_empty() {
            break;
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
    xfer: Option<&XferCtrl>,
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
    xfer: Option<&XferCtrl>,
) -> Result<u64, String> {
    let mut lf = tokio::fs::File::open(local).await.map_err(|e| e.to_string())?;
    let mut rf = sftp.create(remote).await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; SFTP_CHUNK as usize];
    let mut transferred: u64 = 0;
    let mut last_emit: u64 = 0;
    loop {
        gone(alive, xfer)?;
        wait_if_paused(app, alive, xfer, item_id, session_id, "upload", local, remote, rel, size, transferred).await?;
        let n = tokio::select! {
            _ = wait_cancel(alive, xfer) => return Err(CANCELLED.into()),
            n = lf.read(&mut buf) => n.map_err(|e| e.to_string())?,
        };
        if n == 0 {
            break;
        }
        tokio::select! {
            _ = wait_cancel(alive, xfer) => return Err(CANCELLED.into()),
            r = rf.write_all(&buf[..n]) => r.map_err(|e| e.to_string())?,
        }
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
pub(crate) fn emit_transfer(
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
    handle: SharedHandle,
    session_id: &str,
    local: &str,
    remote_dir: &str,
    alive: Arc<AtomicBool>,
    hub: TransferHub,
) -> Result<(), String> {
    gone(Some(&alive), None)?;
    check_remote_path(remote_dir)?;
    let local = local.replace('\\', "/");
    let root_name = Path::new(&local)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let mut files: Vec<(String, String, String, u64)> = Vec::new();
    collect_local(&local, &join_remote(remote_dir, &root_name), &root_name, &mut files).await?;

    let sftp = open(handle.as_ref()).await?;
    let mut parents: Vec<String> = files
        .iter()
        .filter_map(|(_, rp, _, _)| {
            Path::new(rp)
                .parent()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    parents.sort_by_key(|p| p.len());
    parents.dedup();
    for p in parents {
        let _ = ensure_remote_dir(&sftp, &p).await;
    }
    drop(sftp);

    let mut jobs = Vec::new();
    for (lp, rp, rel, size) in files {
        let key = dup_key(session_id, "upload", &lp, &rp);
        let id = uuid::Uuid::new_v4().to_string();
        let Some(ctrl) = hub.start(&id, session_id, key) else {
            continue;
        };
        emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "queued", None);
        jobs.push((id, ctrl, lp, rp, rel, size));
    }

    let session_id = session_id.to_string();
    stream::iter(jobs)
        .map(|(id, ctrl, lp, rp, rel, size)| {
            let app = app.clone();
            let handle = handle.clone();
            let hub = hub.clone();
            let alive = alive.clone();
            let session_id = session_id.clone();
            async move {
                let _permit = hub.slots.acquire().await;
                if !ctrl.is_live() || !alive.load(Ordering::Relaxed) {
                    hub.finish(&id);
                    emit_transfer(
                        &app, &id, &session_id, "upload", &lp, &rp, &rel, size, 0, "canceled", None,
                    );
                    return;
                }
                emit_transfer(&app, &id, &session_id, "upload", &lp, &rp, &rel, size, 0, "active", None);
                let result = match open(handle.as_ref()).await {
                    Ok(sftp) => {
                        copy_local_to_remote(
                            Some(&app),
                            &sftp,
                            &id,
                            &session_id,
                            &lp,
                            &rp,
                            &rel,
                            size,
                            Some(&alive),
                            Some(ctrl.as_ref()),
                        )
                        .await
                    }
                    Err(e) => Err(e),
                };
                hub.finish(&id);
                match result {
                    Ok(_) => emit_transfer(
                        &app, &id, &session_id, "upload", &lp, &rp, &rel, size, size, "done", None,
                    ),
                    Err(e) if e == CANCELLED => emit_transfer(
                        &app, &id, &session_id, "upload", &lp, &rp, &rel, size, 0, "canceled", None,
                    ),
                    Err(e) => emit_transfer(
                        &app, &id, &session_id, "upload", &lp, &rp, &rel, size, 0, "error", Some(&e),
                    ),
                }
            }
        })
        .buffer_unordered(TRANSFER_MAX * 2)
        .for_each(|_| async {})
        .await;
    Ok(())
}

/// Рекурсивно скачивает удалённый путь (файл/папка) в localDir, эмитя события.
pub async fn download_path(
    app: AppHandle,
    handle: SharedHandle,
    session_id: &str,
    remote: &str,
    local_dir: &str,
    alive: Arc<AtomicBool>,
    hub: TransferHub,
) -> Result<(), String> {
    gone(Some(&alive), None)?;
    check_remote_path(remote)?;
    let sftp = open(handle.as_ref()).await?;
    let root_name = Path::new(remote)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".into());
    let local_dir = local_dir.replace('\\', "/");
    let mut files: Vec<(String, String, String, u64)> = Vec::new();
    collect_remote(&sftp, remote, &format!("{local_dir}/{root_name}"), &root_name, &mut files).await?;
    drop(sftp);

    for (lp, _, _, _) in &files {
        if let Some(parent) = Path::new(lp).parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
    }

    let mut jobs = Vec::new();
    for (lp, rp, rel, size) in files {
        if size > 0 {
            if let Ok(meta) = tokio::fs::metadata(&lp).await {
                if meta.is_file() && meta.len() == size {
                    let id = uuid::Uuid::new_v4().to_string();
                    emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, size, "done", None);
                    continue;
                }
            }
        }
        let key = dup_key(session_id, "download", &lp, &rp);
        let id = uuid::Uuid::new_v4().to_string();
        let Some(ctrl) = hub.start(&id, session_id, key) else {
            continue;
        };
        emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "queued", None);
        jobs.push((id, ctrl, lp, rp, rel, size));
    }

    let session_id = session_id.to_string();
    stream::iter(jobs)
        .map(|(id, ctrl, lp, rp, rel, size)| {
            let app = app.clone();
            let handle = handle.clone();
            let hub = hub.clone();
            let alive = alive.clone();
            let session_id = session_id.clone();
            async move {
                let _permit = hub.slots.acquire().await;
                if !ctrl.is_live() || !alive.load(Ordering::Relaxed) {
                    hub.finish(&id);
                    emit_transfer(
                        &app, &id, &session_id, "download", &lp, &rp, &rel, size, 0, "canceled", None,
                    );
                    return;
                }
                emit_transfer(
                    &app, &id, &session_id, "download", &lp, &rp, &rel, size, 0, "active", None,
                );
                let result = match open(handle.as_ref()).await {
                    Ok(sftp) => {
                        copy_remote_to_local(
                            Some(&app),
                            handle.as_ref(),
                            &sftp,
                            &id,
                            &session_id,
                            &rp,
                            &lp,
                            &rel,
                            size,
                            Some(&alive),
                            Some(ctrl.as_ref()),
                        )
                        .await
                    }
                    Err(e) => Err(e),
                };
                hub.finish(&id);
                match result {
                    Ok(n) => emit_transfer(
                        &app, &id, &session_id, "download", &lp, &rp, &rel, size, n, "done", None,
                    ),
                    Err(e) if e == CANCELLED => emit_transfer(
                        &app, &id, &session_id, "download", &lp, &rp, &rel, size, 0, "canceled", None,
                    ),
                    Err(e) => emit_transfer(
                        &app, &id, &session_id, "download", &lp, &rp, &rel, size, 0, "error", Some(&e),
                    ),
                }
            }
        })
        .buffer_unordered(TRANSFER_MAX * 2)
        .for_each(|_| async {})
        .await;
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

/// Имена из names, которые уже есть в remote_dir (для вопроса про замену).
pub async fn name_conflicts(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote_dir: &str,
    names: &[String],
) -> Result<Vec<String>, String> {
    check_remote_path(remote_dir)?;
    let sftp = open(handle).await?;
    let mut out = Vec::new();
    for raw in names {
        let norm = raw.replace('\\', "/");
        let name = norm.rsplit('/').next().unwrap_or(norm.as_str());
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        let p = join_remote(remote_dir, name);
        if sftp.metadata(&p).await.is_ok() {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::check_remote_path;

    #[test]
    fn absolute_paths_pass() {
        assert!(check_remote_path("/srv/site/docker-compose.yml").is_ok());
        assert!(check_remote_path("/").is_ok());
        assert!(check_remote_path("/home/hade/файл с пробелом.txt").is_ok());
        // «..» как часть имени — не выход наверх.
        assert!(check_remote_path("/srv/..hidden").is_ok());
    }

    #[test]
    fn relative_and_empty_rejected() {
        assert!(check_remote_path("").is_err());
        assert!(check_remote_path("   ").is_err());
        assert!(check_remote_path("srv/site").is_err());
        assert!(check_remote_path("~/notes").is_err());
    }

    #[test]
    fn parent_traversal_rejected() {
        assert!(check_remote_path("/srv/../etc/shadow").is_err());
        assert!(check_remote_path("/..").is_err());
        assert!(check_remote_path("/srv/site/..").is_err());
    }

    #[test]
    fn control_chars_rejected() {
        assert!(check_remote_path("/srv/site\nrm -rf /").is_err());
        assert!(check_remote_path("/srv/\u{0}etc").is_err());
    }
}
