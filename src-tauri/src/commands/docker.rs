//! Docker: контейнеры, журналы, статистика, compose.

use crate::{actionlog, docker, docker_compose, ssh, AppState};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

/// Отказ в том же виде, что и неудача на сервере: `{ok: false, error}`. Панель Docker умеет
/// показывать только его - ошибка вызова оставила бы кнопку висеть в состоянии «занято».
fn refused(error: String) -> Value {
    json!({ "ok": false, "error": error })
}

#[tauri::command]
pub async fn docker_list(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let (code, out, err) = ssh::exec(&s.handle, docker::LIST_CMD, Some(s.cancel.subscribe())).await?;
    Ok(docker::parse_list(code, &out, &err))
}

#[tauri::command]
pub async fn docker_action(
    state: State<'_, AppState>,
    id: String,
    container_id: String,
    action: String,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let detail = json!({ "container": container_id, "action": action });
    let cmd = match docker::action_cmd(&container_id, &action) {
        Ok(cmd) => cmd,
        Err(e) => {
            actionlog::record_session(&id, "docker.action", detail, &Err::<(), _>(&e));
            return Ok(refused(e));
        }
    };
    let (code, _o, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    let result = ssh::exit_result(code, &err, || format!("Код {code}"));
    actionlog::record_session(&id, "docker.action", detail, &result);
    Ok(match result {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e }),
    })
}

#[tauri::command]
pub async fn docker_logs(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    container_id: String,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = match docker::logs_cmd(&container_id) {
        Ok(cmd) => cmd,
        Err(e) => return Ok(refused(e)),
    };
    let key = format!("{id}:docker-logs:{container_id}");
    let op = state.ops.begin(&key);
    let cancel = ssh::race_cancel(s.cancel.subscribe(), op);
    let app2 = app.clone();
    let sid = id.clone();
    let cid = container_id.clone();
    let result = ssh::exec_with(&s.handle, &cmd, Some(cancel), move |chunk| {
        if chunk.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(chunk);
        let _ = app2.emit(
            "docker-logs",
            json!({ "sessionId": sid, "containerId": cid, "chunk": text.as_ref() }),
        );
    })
    .await;
    state.ops.finish(&key);
    let (_c, out, _e) = result?;
    Ok(json!({ "ok": true, "logs": out }))
}

/// CPU и память всех работающих контейнеров одним вызовом - для колонок списка.
#[tauri::command]
pub async fn docker_stats_all(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let (code, out, err) = ssh::exec(&s.handle, docker::STATS_ALL_CMD, Some(s.cancel.subscribe())).await?;
    Ok(docker::parse_stats_all(code, &out, &err))
}

#[tauri::command]
pub async fn docker_stats(state: State<'_, AppState>, id: String, container_id: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = match docker::stats_cmd(&container_id) {
        Ok(cmd) => cmd,
        Err(e) => return Ok(refused(e)),
    };
    let (code, out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(docker::parse_stats(code, &out, &err))
}

#[tauri::command]
pub fn docker_logs_cancel(state: State<'_, AppState>, id: String, container_id: Option<String>) {
    match container_id {
        Some(cid) if !cid.is_empty() => state.ops.cancel(&format!("{id}:docker-logs:{cid}")),
        _ => state.ops.cancel_prefix(&format!("{id}:docker-logs:")),
    }
}

#[tauri::command]
pub async fn docker_container_files(
    state: State<'_, AppState>,
    id: String,
    container_id: String,
    path: String,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = match docker::files_cmd(&container_id, &path) {
        Ok(cmd) => cmd,
        Err(e) => return Ok(refused(e)),
    };
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
pub async fn docker_compose_list(state: State<'_, AppState>, id: String) -> Result<Value, String> {
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
pub async fn docker_compose_ps(
    state: State<'_, AppState>,
    id: String,
    compose_file: String,
    project: String,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = match docker_compose::ps_cmd(&compose_file, &project) {
        Ok(cmd) => cmd,
        Err(e) => return Ok(refused(e)),
    };
    match docker_exec(&s.handle, &cmd, Some(s.cancel.subscribe()), 20).await {
        Ok((code, out, err)) => Ok(docker_compose::parse_ps(code, &out, &err)),
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

#[tauri::command]
pub async fn docker_compose_action(
    state: State<'_, AppState>,
    id: String,
    compose_file: String,
    project: String,
    action: String,
    service: Option<String>,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let detail = json!({ "composeFile": compose_file, "project": project, "action": action, "service": service });
    let cmd = match docker_compose::action_cmd(&compose_file, &project, &action, service.as_deref()) {
        Ok(cmd) => cmd,
        Err(e) => {
            actionlog::record_session(&id, "docker.compose", detail, &Err::<(), _>(&e));
            return Ok(refused(e));
        }
    };
    let (code, _o, err) = docker_exec(&s.handle, &cmd, Some(s.cancel.subscribe()), 60).await?;
    let result = ssh::exit_result(code, &err, || format!("Код {code}"));
    actionlog::record_session(&id, "docker.compose", detail, &result);
    Ok(match result {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e }),
    })
}

#[tauri::command]
pub async fn docker_compose_read(
    state: State<'_, AppState>,
    id: String,
    compose_file: String,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = match docker_compose::read_compose_cmd(&compose_file) {
        Ok(cmd) => cmd,
        Err(e) => return Ok(refused(e)),
    };
    let (code, out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(docker_compose::parse_compose_text(code, &out, &err))
}

#[tauri::command]
pub async fn docker_compose_logs(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    compose_file: String,
    project: String,
    service: String,
) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let cmd = match docker_compose::logs_cmd(&compose_file, &project, &service) {
        Ok(cmd) => cmd,
        Err(e) => return Ok(refused(e)),
    };
    let key = format!("{id}:compose-logs:{compose_file}:{service}");
    let op = state.ops.begin(&key);
    let cancel = ssh::race_cancel(s.cancel.subscribe(), op);
    let app2 = app.clone();
    let sid = id.clone();
    let svc = service.clone();
    let cf = compose_file.clone();
    let result = ssh::exec_with(&s.handle, &cmd, Some(cancel), move |chunk| {
        if chunk.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(chunk);
        let _ = app2.emit(
            "docker-logs",
            json!({ "sessionId": sid, "containerId": format!("compose:{cf}:{svc}"), "chunk": text.as_ref() }),
        );
    })
    .await;
    state.ops.finish(&key);
    let (_c, out, _e) = result?;
    Ok(json!({ "ok": true, "logs": out }))
}

#[tauri::command]
pub fn docker_compose_logs_cancel(
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
