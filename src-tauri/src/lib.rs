//! Serein — backend. Команды Tauri и менеджер сессий.

mod backup;
mod clipboard;
mod crypto;
mod dnd;
mod docker;
mod docker_compose;
mod dpapi;
mod importers;
mod keygen;
mod knownhosts;
mod localfs;
mod monitor;
mod pty;
mod remoteedit;
pub mod sftp;
pub mod ssh;
pub mod store;
mod term_out;
mod tunnels;
mod vault;
mod vaultkey;
mod workspace;

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

pub(crate) enum Session {
    Local(pty::LocalSession),
    Ssh(Arc<ssh::SshSession>),
}

pub(crate) struct AppState {
    sessions: Mutex<HashMap<String, Session>>,
    ki: ssh::KiBridge,
    tunnels: tunnels::TunnelManager,
    edit: remoteedit::EditManager,
    transfers: sftp::TransferHub,
    ops: ssh::OpHub,
}

impl AppState {
    fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            ki: Arc::new(Mutex::new(HashMap::new())),
            tunnels: tunnels::TunnelManager::default(),
            edit: remoteedit::EditManager::default(),
            transfers: sftp::TransferHub::default(),
            ops: ssh::OpHub::default(),
        }
    }
    fn ssh(&self, id: &str) -> Option<Arc<ssh::SshSession>> {
        match self.sessions.lock().unwrap().get(id) {
            Some(Session::Ssh(s)) => Some(s.clone()),
            _ => None,
        }
    }

    /// Идемпотентно: туннели, edit-watchers, KI, russh disconnect. Можно звать с фронта и из shell-таска.
    pub(crate) fn teardown(&self, app: &AppHandle, id: &str, user: bool) {
        self.tunnels.close_session(id, app);
        self.edit.stop_session(id);
        self.transfers.cancel_session(id);
        self.ops.cancel_prefix(&format!("{id}:"));
        if let Some(tx) = self.ki.lock().unwrap().remove(id) {
            drop(tx);
        }
        if let Some(s) = self.sessions.lock().unwrap().remove(id) {
            match s {
                Session::Local(l) => l.close(),
                Session::Ssh(s) => s.shutdown(user),
            }
        }
    }
}

fn emit_connected(app: &AppHandle, id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(160)).await;
        let _ = app.emit("session-status", json!({ "id": id, "status": "connected" }));
    });
}

// ---------------- Настройки / серверы / сниппеты / раскладка / localfs ----------------

#[tauri::command]
fn settings_get() -> Value {
    store::settings_get()
}
#[tauri::command]
fn settings_set(state: State<'_, AppState>, patch: Value) -> Result<Value, String> {
    let cur = store::settings_set(patch)?;
    if let Some(n) = cur.get("sftpConcurrency").and_then(|v| v.as_u64()) {
        state.transfers.set_limit(n as usize);
    }
    Ok(cur)
}
#[tauri::command]
fn servers_list() -> Vec<Value> {
    store::servers_list_safe()
}
#[tauri::command]
fn servers_save(cfg: Value) -> Result<Value, String> {
    store::servers_save(cfg)
}
#[tauri::command]
fn servers_delete(id: String) -> Result<(), String> {
    store::servers_delete(&id)
}
#[tauri::command]
fn snippets_list() -> Vec<Value> {
    store::snippets_list()
}
#[tauri::command]
fn snippets_save(s: Value) -> Result<Value, String> {
    store::snippets_save(s)
}
#[tauri::command]
fn snippets_delete(id: String) -> Result<(), String> {
    store::snippets_delete(&id)
}
#[tauri::command]
fn layout_get() -> Value {
    store::layout_get()
}
#[tauri::command]
fn clipboard_write(text: String) -> Result<(), String> {
    clipboard::write_text(&text)
}
#[tauri::command]
fn clipboard_read() -> Result<String, String> {
    clipboard::read_text()
}
#[tauri::command]
fn layout_set(tabs: Value) -> Result<(), String> {
    store::layout_set(tabs)
}
#[tauri::command]
fn aux_layout_get() -> Value {
    store::aux_layout_get()
}
#[tauri::command]
fn aux_layout_set(layout: Value) -> Result<(), String> {
    store::aux_layout_set(layout)
}
#[tauri::command]
fn localfs_home() -> String {
    localfs::home()
}
#[tauri::command]
fn localfs_parent(path: String) -> String {
    localfs::parent(&path)
}
#[tauri::command]
fn localfs_list(path: String) -> Result<Value, String> {
    localfs::list(&path)
}
#[tauri::command]
fn localfs_copy_into(paths: Vec<String>, dest_dir: String) -> Result<u32, String> {
    localfs::copy_into(&paths, &dest_dir)
}

// ---------------- Сессии ----------------

fn resolve_chain(server_id: &str) -> Result<Vec<Value>, String> {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut id = Some(server_id.to_string());
    while let Some(sid) = id {
        if !seen.insert(sid.clone()) {
            return Err("Циклическая цепочка jump-хостов".into());
        }
        let s = store::server_with_secrets(&sid).ok_or("Сервер из цепочки jump-хостов не найден")?;
        let next = s
            .get("proxyJump")
            .and_then(|v| v.as_str())
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string());
        chain.push(s);
        id = next;
    }
    Ok(chain)
}

#[tauri::command]
fn session_open_local(app: AppHandle, state: State<'_, AppState>, p: Value) -> Result<String, String> {
    let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16;
    let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16;
    let cwd = p.get("cwd").and_then(|v| v.as_str()).map(|s| s.to_string());
    let pref = store::settings_get()
        .get("localShell")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .to_string();
    let shell = pty::resolve_shell(&pref);
    let id = uuid::Uuid::new_v4().to_string();
    let sess = pty::open_local(app.clone(), id.clone(), shell, cwd, cols, rows)?;
    state.sessions.lock().unwrap().insert(id.clone(), Session::Local(sess));
    emit_connected(&app, id.clone());
    Ok(id)
}

#[tauri::command]
async fn session_open_ssh(app: AppHandle, state: State<'_, AppState>, p: Value) -> Result<String, String> {
    let server_id = p.get("serverId").and_then(|v| v.as_str()).ok_or("Не задан serverId")?;
    let chain = resolve_chain(server_id)?;
    let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u32;
    let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u32;
    let id = uuid::Uuid::new_v4().to_string();
    let ki = state.ki.clone();

    let sess = ssh::connect_chain(app.clone(), id.clone(), chain, cols, rows, ki).await?;
    let sess = Arc::new(sess);
    // Автозапуск туннелей + команда на подключении.
    let server = store::server_with_secrets(server_id);
    if let Some(srv) = &server {
        if let Some(tunnels) = srv.get("tunnels").and_then(|v| v.as_array()) {
            for t in tunnels {
                let _ = state
                    .tunnels
                    .open(app.clone(), sess.handle.clone(), id.clone(), t.clone(), sess.remote_forwards.clone(), sess.cancel.subscribe())
                    .await;
            }
        }
        if let Some(cmd) = srv.get("executeOnConnect").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            let _ = sess.tx.send(ssh::SshCmd::Write(format!("{cmd}\r").into_bytes()));
        }
    }
    state.sessions.lock().unwrap().insert(id.clone(), Session::Ssh(sess));
    emit_connected(&app, id.clone());
    Ok(id)
}

#[tauri::command]
fn session_write(state: State<'_, AppState>, id: String, data: String) {
    if let Some(s) = state.sessions.lock().unwrap().get(&id) {
        match s {
            Session::Local(l) => l.write(&data),
            Session::Ssh(s) => {
                let _ = s.tx.send(ssh::SshCmd::Write(data.into_bytes()));
            }
        }
    }
}

#[tauri::command]
fn session_resize(state: State<'_, AppState>, p: Value) {
    let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(80);
    let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(24);
    if cols < 20 || rows < 5 {
        return;
    }
    if let Some(s) = state.sessions.lock().unwrap().get(&id) {
        match s {
            Session::Local(l) => l.resize(cols as u16, rows as u16),
            Session::Ssh(s) => {
                let _ = s.tx.send(ssh::SshCmd::Resize(cols as u32, rows as u32));
            }
        }
    }
}

#[tauri::command]
fn session_close(app: AppHandle, state: State<'_, AppState>, id: String) {
    state.teardown(&app, &id, true);
}

#[tauri::command]
async fn session_ping(state: State<'_, AppState>, id: String) -> Result<Option<u32>, String> {
    match state.ssh(&id) {
        Some(s) => Ok(ssh::ping(&s.handle).await),
        None => Ok(None),
    }
}

#[tauri::command]
fn session_log_status(id: String) -> bool {
    term_out::log_active(&id)
}

/// Включает или выключает запись вывода сессии в файл (`%APPDATA%\serein\logs`).
#[tauri::command]
fn session_log_toggle(id: String, title: String) -> Result<Value, String> {
    if term_out::log_active(&id) {
        let path = term_out::log_path_of(&id);
        term_out::log_stop(&id);
        return Ok(json!({ "logging": false, "path": path }));
    }
    let path = term_out::log_start(&id, &title)?;
    Ok(json!({ "logging": true, "path": path }))
}

#[tauri::command]
async fn session_monitor(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let (_c, out, _e) = ssh::exec(&s.handle, monitor::SAMPLE_CMD, Some(s.cancel.subscribe())).await?;
    Ok(monitor::parse(&out))
}

#[tauri::command]
async fn workspace_processes(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (code, out, err) = ssh::exec(&s.handle, workspace::PS_CMD, Some(s.cancel.subscribe())).await?;
    if code != 0 && out.trim().is_empty() {
        let error = if err.trim().is_empty() {
            "ps недоступен".to_string()
        } else {
            err.trim().to_string()
        };
        return Ok(json!({ "ok": false, "error": error }));
    }
    Ok(workspace::parse_ps(&out))
}

#[tauri::command]
async fn workspace_kill(state: State<'_, AppState>, session_id: String, pid: u32) -> Result<Value, String> {
    let cmd = workspace::kill_cmd(pid)?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (code, _out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    if code != 0 {
        let error = if err.trim().is_empty() {
            format!("kill завершился с кодом {code}")
        } else {
            err.trim().to_string()
        };
        return Ok(json!({ "ok": false, "error": error }));
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
async fn workspace_services(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (code, out, err) = ssh::exec(&s.handle, workspace::SERVICES_CMD, Some(s.cancel.subscribe())).await?;
    Ok(workspace::parse_services(code, &out, &err))
}

#[tauri::command]
async fn workspace_service_action(
    state: State<'_, AppState>,
    session_id: String,
    name: String,
    action: String,
) -> Result<Value, String> {
    let cmd = workspace::service_cmd(&name, &action)?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (code, _out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    if code != 0 {
        let error = if err.trim().is_empty() {
            format!("systemctl {action} код {code}")
        } else {
            err.trim().to_string()
        };
        return Ok(json!({ "ok": false, "error": error }));
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
async fn workspace_logs(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (_code, out, err) = ssh::exec(&s.handle, workspace::LOGS_CMD, Some(s.cancel.subscribe())).await?;
    let text = if out.trim().is_empty() { err } else { out };
    Ok(json!({ "ok": true, "text": text }))
}

#[tauri::command]
fn session_ki_respond(state: State<'_, AppState>, id: String, answers: Vec<String>) {
    if let Some(tx) = state.ki.lock().unwrap().remove(&id) {
        let _ = tx.send(answers);
    }
}

// ---------------- Docker ----------------

#[tauri::command]
async fn docker_list(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let (code, out, err) = ssh::exec(&s.handle, docker::LIST_CMD, Some(s.cancel.subscribe())).await?;
    Ok(docker::parse_list(code, &out, &err))
}
#[tauri::command]
async fn docker_action(state: State<'_, AppState>, id: String, container_id: String, action: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = docker::action_cmd(&container_id, &action).ok_or_else(|| format!("Неизвестное действие: {action}"))?;
    let (code, _o, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    if code != 0 {
        Ok(json!({ "ok": false, "error": if err.trim().is_empty() { format!("Код {code}") } else { err.trim().to_string() } }))
    } else {
        Ok(json!({ "ok": true }))
    }
}
#[tauri::command]
async fn docker_logs(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    container_id: String,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let key = format!("{id}:docker-logs:{container_id}");
    let op = state.ops.begin(&key);
    let cancel = ssh::race_cancel(s.cancel.subscribe(), op);
    let app2 = app.clone();
    let sid = id.clone();
    let cid = container_id.clone();
    let result = ssh::exec_with(
        &s.handle,
        &docker::logs_cmd(&container_id),
        Some(cancel),
        move |chunk| {
            if chunk.is_empty() {
                return;
            }
            let text = String::from_utf8_lossy(chunk);
            let _ = app2.emit(
                "docker-logs",
                json!({ "sessionId": sid, "containerId": cid, "chunk": text.as_ref() }),
            );
        },
    )
    .await;
    state.ops.finish(&key);
    let (_c, out, _e) = result?;
    Ok(json!({ "ok": true, "logs": out }))
}

#[tauri::command]
async fn docker_stats(state: State<'_, AppState>, id: String, container_id: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let (code, out, err) = ssh::exec(&s.handle, &docker::stats_cmd(&container_id), Some(s.cancel.subscribe())).await?;
    Ok(docker::parse_stats(code, &out, &err))
}

#[tauri::command]
fn docker_logs_cancel(state: State<'_, AppState>, id: String, container_id: Option<String>) {
    match container_id {
        Some(cid) if !cid.is_empty() => state.ops.cancel(&format!("{id}:docker-logs:{cid}")),
        _ => state.ops.cancel_prefix(&format!("{id}:docker-logs:")),
    }
}

#[tauri::command]
async fn docker_container_files(
    state: State<'_, AppState>,
    id: String,
    container_id: String,
    path: String,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = docker::files_cmd(&container_id, &path).ok_or("Недопустимый путь")?;
    let (code, out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(docker::parse_files(code, &out, &err, &path))
}

async fn docker_exec(
    handle: &ssh::SharedHandle,
    cmd: &str,
    cancel: Option<ssh::CancelRx>,
    secs: u64,
) -> Result<(i32, String, String), String> {
    match tokio::time::timeout(std::time::Duration::from_secs(secs), ssh::exec(handle, cmd, cancel)).await {
        Ok(r) => r,
        Err(_) => Err(format!("Таймаут команды ({secs} с)")),
    }
}

#[tauri::command]
async fn docker_compose_list(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cancel = Some(s.cancel.subscribe());
    let primary = match docker_exec(&s.handle, docker_compose::LIST_CMD, cancel.clone(), 15).await {
        Ok((code, out, err)) => docker_compose::parse_list(code, &out, &err),
        Err(e) => json!({ "ok": false, "error": e }),
    };
    if primary["projects"].as_array().map(|a| !a.is_empty()).unwrap_or(false) {
        return Ok(primary);
    }
    let fallback = match docker_exec(&s.handle, docker_compose::LIST_PS_JSON_CMD, cancel, 15).await {
        Ok((code, out, err)) => docker_compose::parse_list_from_ps_json(code, &out, &err),
        Err(e) => json!({ "ok": false, "error": e }),
    };
    Ok(docker_compose::merge_projects(primary, fallback))
}

#[tauri::command]
async fn docker_compose_ps(
    state: State<'_, AppState>,
    id: String,
    compose_file: String,
    project: String,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = docker_compose::ps_cmd(&compose_file, &project).ok_or("Недопустимые параметры compose")?;
    match docker_exec(&s.handle, &cmd, Some(s.cancel.subscribe()), 20).await {
        Ok((code, out, err)) => Ok(docker_compose::parse_ps(code, &out, &err)),
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

#[tauri::command]
async fn docker_compose_action(
    state: State<'_, AppState>,
    id: String,
    compose_file: String,
    project: String,
    action: String,
    service: Option<String>,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let svc = service.as_deref();
    let cmd = docker_compose::action_cmd(&compose_file, &project, &action, svc)
        .ok_or_else(|| format!("Неизвестное действие: {action}"))?;
    let (code, _o, err) = docker_exec(&s.handle, &cmd, Some(s.cancel.subscribe()), 60).await?;
    if code != 0 {
        Ok(json!({ "ok": false, "error": if err.trim().is_empty() { format!("Код {code}") } else { err.trim().to_string() } }))
    } else {
        Ok(json!({ "ok": true }))
    }
}

#[tauri::command]
async fn docker_compose_read(state: State<'_, AppState>, id: String, compose_file: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = docker_compose::read_compose_cmd(&compose_file).ok_or("Недопустимый compose-файл")?;
    let (code, out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(docker_compose::parse_compose_text(code, &out, &err))
}

#[tauri::command]
async fn docker_compose_logs(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    compose_file: String,
    project: String,
    service: String,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = docker_compose::logs_cmd(&compose_file, &project, &service).ok_or("Недопустимые параметры")?;
    let key = format!("{id}:compose-logs:{compose_file}:{service}");
    let op = state.ops.begin(&key);
    let cancel = ssh::race_cancel(s.cancel.subscribe(), op);
    let app2 = app.clone();
    let sid = id.clone();
    let svc = service.clone();
    let cf = compose_file.clone();
    let result = ssh::exec_with(
        &s.handle,
        &cmd,
        Some(cancel),
        move |chunk| {
            if chunk.is_empty() {
                return;
            }
            let text = String::from_utf8_lossy(chunk);
            let _ = app2.emit(
                "docker-logs",
                json!({ "sessionId": sid, "containerId": format!("compose:{cf}:{svc}"), "chunk": text.as_ref() }),
            );
        },
    )
    .await;
    state.ops.finish(&key);
    let (_c, out, _e) = result?;
    Ok(json!({ "ok": true, "logs": out }))
}

#[tauri::command]
fn docker_compose_logs_cancel(
    state: State<'_, AppState>,
    id: String,
    compose_file: Option<String>,
    service: Option<String>,
) {
    match (compose_file, service) {
        (Some(cf), Some(svc)) if !cf.is_empty() && !svc.is_empty() => {
            state.ops.cancel(&format!("{id}:compose-logs:{cf}:{svc}"))
        }
        _ => state.ops.cancel_prefix(&format!("{id}:compose-logs:")),
    }
}

// ---------------- SFTP ----------------

#[tauri::command]
async fn sftp_list(state: State<'_, AppState>, session_id: String, path: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    sftp::list(&s.handle, &path).await
}
#[tauri::command]
async fn sftp_mkdir(state: State<'_, AppState>, session_id: String, path: String) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    sftp::mkdir(&s.handle, &path).await
}
#[tauri::command]
async fn sftp_remove(state: State<'_, AppState>, session_id: String, path: String, is_dir: bool) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    sftp::remove(&s.handle, &path, is_dir).await
}
#[tauri::command]
async fn sftp_rename(state: State<'_, AppState>, session_id: String, from: String, to: String) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    sftp::rename(&s.handle, &from, &to).await
}
#[tauri::command]
async fn sftp_chmod(state: State<'_, AppState>, session_id: String, path: String, mode: u32) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    sftp::chmod(&s.handle, &path, mode).await
}
#[tauri::command]
async fn sftp_preview(state: State<'_, AppState>, session_id: String, remote_path: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    sftp::preview(&s.handle, &remote_path).await
}
#[tauri::command]
async fn sftp_read_file(state: State<'_, AppState>, session_id: String, remote_path: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    sftp::read_file(&s.handle, &remote_path).await
}
#[tauri::command]
async fn sftp_write_file(state: State<'_, AppState>, session_id: String, remote_path: String, content: String, mode: u32, base_mtime: u64, eol: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    sftp::write_file(&s.handle, &remote_path, &content, mode, base_mtime, &eol).await
}
#[tauri::command]
async fn sftp_name_conflicts(
    state: State<'_, AppState>,
    session_id: String,
    remote_dir: String,
    names: Vec<String>,
) -> Result<Vec<String>, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    sftp::name_conflicts(&s.handle, &remote_dir, &names).await
}
#[tauri::command]
async fn sftp_upload_paths(app: AppHandle, state: State<'_, AppState>, session_id: String, remote_dir: String, paths: Vec<String>) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let n = paths.len();
    let handle = s.handle.clone();
    let alive = s.alive.clone();
    let hub = state.transfers.clone();
    let futs: Vec<_> = paths
        .into_iter()
        .map(|p| {
            let app = app.clone();
            let handle = handle.clone();
            let sid = session_id.clone();
            let remote = remote_dir.clone();
            let alive = alive.clone();
            let hub = hub.clone();
            async move { sftp::upload_path(app, handle, &sid, &p, &remote, alive, hub).await }
        })
        .collect();
    futures::future::join_all(futs).await;
    Ok(json!({ "uploaded": n }))
}
#[tauri::command]
async fn sftp_download_to(app: AppHandle, state: State<'_, AppState>, session_id: String, remote_path: String, local_dir: String) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    sftp::download_path(app, s.handle.clone(), &session_id, &remote_path, &local_dir, s.alive.clone(), state.transfers.clone()).await
}
#[tauri::command]
async fn sftp_drag_out(
    window: tauri::WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    remote_paths: Vec<String>,
) -> Result<(), String> {
    if remote_paths.is_empty() {
        return Err("Нечего перетаскивать".into());
    }
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let handle = s.handle.clone();
    let alive = s.alive.clone();
    let hub = state.transfers.clone();
    // Не ждём скачивание в invoke с dragstart: WebView2 может оборвать промис и дропнуть SFTP-канал.
    tauri::async_runtime::spawn(async move {
        dnd::cleanup_old();
        let mut ole_ok = true;
        if let Ok(sftp) = sftp::open(&handle).await {
            let mut total = 0u64;
            for remote in &remote_paths {
                match sftp.metadata(remote).await {
                    Ok(m) if m.file_type().is_dir() => ole_ok = false,
                    Ok(m) => total = total.saturating_add(m.size.unwrap_or(0)),
                    Err(_) => ole_ok = false,
                }
            }
            if total > dnd::OLE_MAX_BYTES {
                ole_ok = false;
            }
        } else {
            ole_ok = false;
        }
        let dest_dir = if ole_ok {
            match dnd::new_tmp() {
                Ok(t) => t,
                Err(_) => return,
            }
        } else {
            dnd::downloads_dir()
        };
        let dest_s = dest_dir.to_string_lossy().replace('\\', "/");
        for remote in &remote_paths {
            let _ = sftp::download_path(
                app.clone(),
                handle.clone(),
                &session_id,
                remote,
                &dest_s,
                alive.clone(),
                hub.clone(),
            )
            .await;
        }
        if !ole_ok {
            return;
        }
        let mut locals = Vec::new();
        for remote in &remote_paths {
            let Some(name) = std::path::Path::new(remote)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
            else {
                continue;
            };
            let dest = dest_dir.join(&name);
            if !dest.exists() {
                let _ = std::fs::create_dir_all(&dest);
            }
            if let Ok(p) = dnd::drag_path(&dest) {
                locals.push(p);
            }
        }
        if locals.is_empty() {
            return;
        }
        if dnd::start_files(&window, locals, dest_dir.clone()).is_err() {
            let _ = dnd::move_into_downloads(&dest_dir);
        }
    });
    Ok(())
}
#[tauri::command]
async fn sftp_edit(app: AppHandle, state: State<'_, AppState>, session_id: String, remote_path: String) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    state.edit.open(app, s.handle.clone(), session_id, remote_path).await
}
#[tauri::command]
fn sftp_cancel_transfer(state: State<'_, AppState>, id: String) {
    let _ = state.transfers.cancel(&id);
}
#[tauri::command]
fn sftp_pause_transfer(state: State<'_, AppState>, id: String) {
    let _ = state.transfers.pause(&id);
}
#[tauri::command]
fn sftp_resume_transfer(state: State<'_, AppState>, id: String) {
    let _ = state.transfers.resume(&id);
}
#[tauri::command]
fn sftp_edit_stop(app: AppHandle, state: State<'_, AppState>, session_id: String, remote_path: String) {
    state.edit.stop(&app, &session_id, &remote_path);
}

// ---------------- Туннели ----------------

#[tauri::command]
fn tunnel_list_status(state: State<'_, AppState>, session_id: String) -> Vec<Value> {
    state.tunnels.list_status(&session_id)
}
#[tauri::command]
async fn tunnel_open(app: AppHandle, state: State<'_, AppState>, session_id: String, tunnel_id: String) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let server = store::server_with_secrets(&s.server_id).ok_or("Сервер не найден")?;
    let cfg = server
        .get("tunnels")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.iter().find(|t| t.get("id").and_then(|v| v.as_str()) == Some(tunnel_id.as_str())))
        .cloned()
        .ok_or("Конфиг туннеля не найден")?;
    state.tunnels.open(app, s.handle.clone(), session_id, cfg, s.remote_forwards.clone(), s.cancel.subscribe()).await
}
#[tauri::command]
fn tunnel_close(app: AppHandle, state: State<'_, AppState>, session_id: String, tunnel_id: String) {
    state.tunnels.close(&session_id, &tunnel_id, &app);
}

// ---------------- Мастер-пароль / бэкап / keygen / импорт ----------------

#[tauri::command]
fn vault_status() -> Value {
    vault::status()
}
#[tauri::command]
fn vault_unlock(password: String) -> bool {
    vault::unlock(&password)
}
#[tauri::command]
fn vault_enable(password: String) -> Value {
    vault::enable(&password)
}
#[tauri::command]
fn vault_disable(password: String) -> Value {
    vault::disable(&password)
}

#[tauri::command]
fn backup_export(password: String, path: String) -> Result<Value, String> {
    let content = backup::export(&password)?;
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(json!({ "saved": true, "path": path }))
}
#[tauri::command]
fn backup_import(password: String, path: String) -> Result<Value, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let counts = backup::import(&content, &password)?;
    Ok(json!({
        "imported": true,
        "servers": counts.get("servers"),
        "snippets": counts.get("snippets"),
    }))
}

#[tauri::command]
fn export_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
fn keygen_generate(params: Value) -> Result<Value, String> {
    keygen::generate(&params)
}
#[tauri::command]
fn keygen_save(path: String, key: Value) -> Result<Value, String> {
    keygen::save_to(&path, &key)
}
#[tauri::command]
async fn keygen_install(state: State<'_, AppState>, session_id: String, public_key: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (code, _o, err) = ssh::exec(&s.handle, &keygen::install_cmd(&public_key), Some(s.cancel.subscribe())).await?;
    if code != 0 {
        return Err(if err.trim().is_empty() { format!("Код {code}") } else { err.trim().to_string() });
    }
    Ok(json!({ "installed": true }))
}

#[tauri::command]
fn servers_import_ssh_config() -> Result<Value, String> {
    Ok(json!({ "imported": importers::import_ssh_config()? }))
}
#[tauri::command]
fn servers_import_putty() -> Result<Value, String> {
    Ok(json!({ "imported": importers::import_putty()? }))
}

/// Поднять все окна приложения над чужими, фокус оставить на `focused`.
#[tauri::command]
fn windows_raise_group(app: AppHandle, focused: String) {
    #[cfg(windows)]
    windows_raise_group_impl(&app, &focused);
    #[cfg(not(windows))]
    let _ = (app, focused);
}

/// Развернуть все свёрнутые окна приложения (для режима «одна кнопка на панели задач»).
#[tauri::command]
fn windows_restore_minimized(app: AppHandle) -> u32 {
    #[cfg(windows)]
    return windows_restore_minimized_impl(&app);
    #[cfg(not(windows))]
    {
        let _ = app;
        0
    }
}

/// Сколько окон приложения сейчас свёрнуто.
#[tauri::command]
fn windows_count_minimized(app: AppHandle) -> u32 {
    #[cfg(windows)]
    return windows_count_minimized_impl(&app);
    #[cfg(not(windows))]
    {
        let _ = app;
        0
    }
}

#[cfg(windows)]
fn windows_count_minimized_impl(app: &AppHandle) -> u32 {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsIconic;

    let mut n = 0u32;
    for (_, w) in app.webview_windows() {
        let Ok(h) = w.hwnd() else { continue };
        let hwnd = HWND(h.0 as isize as *mut c_void);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                n += 1;
            }
        }
    }
    n
}

#[cfg(windows)]
fn windows_restore_minimized_impl(app: &AppHandle) -> u32 {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, ShowWindow, SW_RESTORE};

    let mut n = 0u32;
    for (_, w) in app.webview_windows() {
        let Ok(h) = w.hwnd() else { continue };
        let hwnd = HWND(h.0 as isize as *mut c_void);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                n += 1;
            }
        }
    }
    n
}

#[cfg(windows)]
fn windows_raise_group_impl(app: &AppHandle, focused: &str) {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SetForegroundWindow, SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    let mut others: Vec<HWND> = Vec::new();
    let mut focus_hwnd: Option<HWND> = None;

    for (label, w) in app.webview_windows() {
        let Ok(h) = w.hwnd() else { continue };
        let hwnd = HWND(h.0 as isize as *mut c_void);
        // Свернутые окна не трогаем — иначе minimize сразу отменяется raise_group.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                continue;
            }
        }
        if label == focused {
            focus_hwnd = Some(hwnd);
        } else {
            others.push(hwnd);
        }
    }

    unsafe {
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        for hwnd in others {
            let _ = SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, flags);
        }
        if let Some(hwnd) = focus_hwnd {
            let _ = SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, flags);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// WebView2 по умолчанию вешает Ctrl+Shift+C на Inspect — это ломает копирование в терминале.
#[cfg(windows)]
fn disable_browser_accelerators(w: &tauri::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
    use windows_core::Interface;
    let _ = w.with_webview(|wv| {
        let controller = wv.controller();
        unsafe {
            let Ok(core) = controller.CoreWebView2() else { return };
            let Ok(settings) = core.Settings() else { return };
            let Ok(s3) = settings.cast::<ICoreWebView2Settings3>() else { return };
            let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false);
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            app.manage(AppState::new());
            #[cfg(windows)]
            if let Some(w) = app.get_webview_window("main") {
                disable_browser_accelerators(&w);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings_get, settings_set,
            servers_list, servers_save, servers_delete,
            snippets_list, snippets_save, snippets_delete,
            layout_get, layout_set, aux_layout_get, aux_layout_set,
            localfs_home, localfs_parent, localfs_list, localfs_copy_into,
            session_open_local, session_open_ssh, session_write, session_resize, session_close,
            session_ping, session_monitor, session_ki_respond,
            session_log_status, session_log_toggle,
            docker_list, docker_action, docker_logs, docker_stats, docker_logs_cancel, docker_container_files,
            docker_compose_list, docker_compose_ps, docker_compose_action, docker_compose_read,
            docker_compose_logs, docker_compose_logs_cancel,
            sftp_list, sftp_mkdir, sftp_remove, sftp_rename, sftp_chmod, sftp_preview, sftp_read_file, sftp_write_file,
            sftp_upload_paths, sftp_download_to, sftp_drag_out, sftp_name_conflicts, sftp_edit, sftp_edit_stop, sftp_cancel_transfer,
            sftp_pause_transfer, sftp_resume_transfer,
            tunnel_list_status, tunnel_open, tunnel_close,
            workspace_processes, workspace_kill, workspace_services, workspace_service_action, workspace_logs,
            vault_status, vault_unlock, vault_enable, vault_disable,
            backup_export, backup_import,
            export_text_file,
            keygen_generate, keygen_save, keygen_install,
            servers_import_ssh_config, servers_import_putty,
            windows_raise_group, windows_restore_minimized, windows_count_minimized,
            clipboard_write, clipboard_read
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
