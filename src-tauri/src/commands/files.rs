//! Файлы: локальная панель, SFTP, передачи, правка удалённых файлов.

use crate::{actionlog, dnd, foldersync, localfs, remote_fs, sftp, ssh, AppState};
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
    let ctx = transfer_ctx(app, &state, &s, &session_id);
    let n = paths.len();
    // Каждый файл виден в списке передач; в журнал - что и куда отправлено и чем кончилось.
    let journal = json!({ "remoteDir": remote_dir, "paths": paths.iter().take(50).collect::<Vec<_>>(), "count": n });
    let results = futures::future::join_all(paths.iter().map(|p| remote_fs::upload_path(&ctx, p, &remote_dir))).await;
    actionlog::record_session(
        &session_id,
        "file.upload",
        journal,
        &remote_fs::batch_outcome_of(&results),
    );
    let failed = results
        .iter()
        .filter(|r| r.as_ref().map_err(Clone::clone).and_then(sftp::Batch::outcome).is_err())
        .count();
    Ok(json!({ "uploaded": n - failed, "failed": failed }))
}

/// Контекст передачи для команд этой сессии.
fn transfer_ctx(app: AppHandle, state: &AppState, s: &ssh::SshSession, session_id: &str) -> remote_fs::Ctx {
    remote_fs::Ctx {
        app,
        fs: s.remote_fs.clone(),
        handle: s.handle.clone(),
        session_id: session_id.to_owned(),
        alive: s.alive.clone(),
        hub: state.transfers.clone(),
    }
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
    let ctx = transfer_ctx(app, &state, &s, &session_id);
    let r = remote_fs::download_path(&ctx, &remote_path, &local_dir).await;
    actionlog::record_session(
        &session_id,
        "file.download",
        json!({ "remotePath": remote_path, "localDir": local_dir }),
        &r.as_ref().map_err(Clone::clone).and_then(sftp::Batch::outcome),
    );
    // Панели - как раньше: сбои отдельных файлов она видит в очереди передач.
    r.map(|_| ())
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
    let ctx = transfer_ctx(app, &state, &s, &session_id);
    // Не ждём скачивание в invoke с dragstart: WebView2 может оборвать промис и дропнуть SFTP-канал.
    tauri::async_runtime::spawn(async move {
        dnd::cleanup_old();
        // Мелкие файлы - OLE-перетаскиванием в Проводник, остальное - сразу в Загрузки.
        let ole = match sftp::open(&ctx.handle).await {
            Ok(sftp) => {
                let mut items = Vec::with_capacity(remote_paths.len());
                for remote in &remote_paths {
                    let meta = sftp.metadata(remote).await.ok();
                    items.push(meta.map(|m| (m.file_type().is_dir(), m.size.unwrap_or(0))));
                }
                dnd::fits_ole(&items)
            }
            Err(_) => false,
        };
        let dest_dir = if ole {
            match dnd::new_tmp() {
                Ok(t) => t,
                Err(_) => return,
            }
        } else {
            dnd::downloads_dir()
        };
        let dest_s = dest_dir.to_string_lossy().replace('\\', "/");
        let mut results = Vec::with_capacity(remote_paths.len());
        for remote in &remote_paths {
            results.push(remote_fs::download_path(&ctx, remote, &dest_s).await);
        }
        // Скачивание перетаскиванием - такое же скачивание, как кнопкой: раньше оно в журнал
        // не попадало вовсе.
        actionlog::record_session(
            &ctx.session_id,
            "file.download",
            json!({
                "remotePaths": remote_paths.iter().take(50).collect::<Vec<_>>(),
                "count": remote_paths.len(),
                "localDir": dest_s,
                "via": "drag",
            }),
            &remote_fs::batch_outcome_of(&results),
        );
        if !ole {
            return;
        }
        let locals = dnd::drag_items(&dest_dir, &remote_paths);
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
