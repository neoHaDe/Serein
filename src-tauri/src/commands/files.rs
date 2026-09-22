//! Файлы: локальная панель, SFTP, передачи, правка удалённых файлов.

use crate::{actionlog, dnd, foldersync, localfs, localname, remote_fs, sftp, AppState};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

#[tauri::command]
pub fn localfs_home() -> String {
    localfs::home()
}

#[tauri::command]
pub fn localfs_parent(path: String) -> String {
    localfs::parent(&path)
}

#[tauri::command]
pub fn localfs_list(path: String) -> Result<Value, String> {
    localfs::list(&path)
}

#[tauri::command]
pub fn localfs_copy_into(paths: Vec<String>, dest_dir: String) -> Result<u32, String> {
    localfs::copy_into(&paths, &dest_dir)
}

#[tauri::command]
pub async fn sftp_list(state: State<'_, AppState>, session_id: String, path: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::list(&s.remote_fs, &s.handle, &path).await
}

/// Сравнивает свою папку с папкой на сервере. Ничего не пишет - это пробный прогон
/// синхронизации; заливка изменённого идёт обычной очередью передач.
#[tauri::command]
pub async fn sftp_compare(
    state: State<'_, AppState>,
    session_id: String,
    local_dir: String,
    remote_dir: String,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    foldersync::compare(&s.remote_fs, &s.handle, &local_dir, &remote_dir, s.alive.clone()).await
}

#[tauri::command]
pub async fn sftp_mkdir(state: State<'_, AppState>, session_id: String, path: String) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let r = remote_fs::mkdir(&s.remote_fs, &s.handle, &path).await;
    actionlog::record_session(&session_id, "file.mkdir", json!({ "path": path }), &r);
    r
}

#[tauri::command]
pub async fn sftp_remove(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
    is_dir: bool,
) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let r = remote_fs::remove(&s.remote_fs, &s.handle, &path, is_dir).await;
    actionlog::record_session(&session_id, "file.remove", json!({ "path": path, "dir": is_dir }), &r);
    r
}

#[tauri::command]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    session_id: String,
    from: String,
    to: String,
) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let r = remote_fs::rename(&s.remote_fs, &s.handle, &from, &to).await;
    actionlog::record_session(&session_id, "file.rename", json!({ "from": from, "to": to }), &r);
    r
}

#[tauri::command]
pub async fn sftp_chmod(state: State<'_, AppState>, session_id: String, path: String, mode: u32) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let r = remote_fs::chmod(&s.remote_fs, &s.handle, &path, mode).await;
    actionlog::record_session(
        &session_id,
        "file.chmod",
        json!({ "path": path, "mode": format!("{mode:o}") }),
        &r,
    );
    r
}

#[tauri::command]
pub async fn sftp_preview(
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::preview(&s.remote_fs, &s.handle, &remote_path).await
}

#[tauri::command]
pub async fn sftp_read_file(
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::read_file(&s.remote_fs, &s.handle, &remote_path).await
}

#[tauri::command]
pub async fn sftp_write_file(
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
    content: String,
    mode: u32,
    base_mtime: u64,
    eol: String,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let r = remote_fs::write_file(&s.remote_fs, &s.handle, &remote_path, &content, mode, base_mtime, &eol).await;
    actionlog::record_session(
        &session_id,
        "file.save",
        json!({ "path": remote_path, "bytes": content.len() }),
        &r,
    );
    r
}

#[tauri::command]
pub async fn sftp_name_conflicts(
    state: State<'_, AppState>,
    session_id: String,
    remote_dir: String,
    names: Vec<String>,
) -> Result<Vec<String>, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::name_conflicts(&s.remote_fs, &s.handle, &remote_dir, &names).await
}

#[tauri::command]
pub async fn sftp_upload_paths(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    remote_dir: String,
    paths: Vec<String>,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let n = paths.len();
    // Каждый файл виден в списке передач; в журнал - что и куда отправлено.
    let journal = json!({ "remoteDir": remote_dir, "paths": paths.iter().take(50).collect::<Vec<_>>(), "count": n });
    let handle = s.handle.clone();
    let remote_fs = s.remote_fs.clone();
    let alive = s.alive.clone();
    let hub = state.transfers.clone();
    let futs: Vec<_> = paths
        .into_iter()
        .map(|p| {
            let app = app.clone();
            let handle = handle.clone();
            let remote_fs = remote_fs.clone();
            let sid = session_id.clone();
            let remote = remote_dir.clone();
            let alive = alive.clone();
            let hub = hub.clone();
            async move { remote_fs::upload_path(app, remote_fs, handle, &sid, &p, &remote, alive, hub).await }
        })
        .collect();
    futures::future::join_all(futs).await;
    actionlog::record_session(&session_id, "file.upload", journal, &Ok::<(), String>(()));
    Ok(json!({ "uploaded": n }))
}

#[tauri::command]
pub async fn sftp_download_to(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
    local_dir: String,
) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let r = remote_fs::download_path(
        app,
        s.remote_fs.clone(),
        s.handle.clone(),
        &session_id,
        &remote_path,
        &local_dir,
        s.alive.clone(),
        state.transfers.clone(),
    )
    .await;
    actionlog::record_session(
        &session_id,
        "file.download",
        json!({ "remotePath": remote_path, "localDir": local_dir }),
        &r,
    );
    r
}

#[tauri::command]
pub async fn sftp_drag_out(
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
    let remote_fs = s.remote_fs.clone();
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
            let _ = remote_fs::download_path(
                app.clone(),
                remote_fs.clone(),
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
            // Имя пришло с сервера: в `join` оно не должно уметь увести за каталог.
            if localname::safe_component(&name).is_err() {
                continue;
            }
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
pub async fn sftp_edit(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let journal = (session_id.clone(), json!({ "remotePath": remote_path }));
    let r = state
        .edit
        .open(app, s.handle.clone(), s.remote_fs.clone(), session_id, remote_path)
        .await;
    actionlog::record_session(&journal.0, "file.edit", journal.1, &r);
    r
}

#[tauri::command]
pub fn sftp_cancel_transfer(state: State<'_, AppState>, id: String) {
    let _ = state.transfers.cancel(&id);
}

#[tauri::command]
pub fn sftp_pause_transfer(state: State<'_, AppState>, id: String) {
    let _ = state.transfers.pause(&id);
}

#[tauri::command]
pub fn sftp_resume_transfer(state: State<'_, AppState>, id: String) {
    let _ = state.transfers.resume(&id);
}

#[tauri::command]
pub fn sftp_edit_stop(app: AppHandle, state: State<'_, AppState>, session_id: String, remote_path: String) {
    state.edit.stop(&app, &session_id, &remote_path);
}
