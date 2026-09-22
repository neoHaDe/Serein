//! Хост за сессией: платформа, процессы, службы, системный журнал.

use crate::{actionlog, platform, ssh, workspace, AppState};
use serde_json::{json, Value};
use tauri::State;

/// Какая система на сервере. Определяется один раз за сессию и кэшируется.
#[tauri::command]
pub async fn workspace_platform(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, version) = platform::of_session(&session_id, &s.handle).await;
    Ok(platform::to_json(kind, &version))
}

#[tauri::command]
pub async fn workspace_processes(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    // Набор команд зависит от системы: `ps` на Windows не существует, и слать его туда
    // значит показать пользователю ошибку вместо таблицы процессов.
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = match kind {
        // Сценарий PowerShell кодируется: в открытом виде его портит оболочка сервера,
        // и на живой проверке команда рвалась ровно посередине.
        platform::Kind::Windows => platform::ps(platform::cmd::PS_WINDOWS),
        platform::Kind::BusyBox => platform::cmd::PS_BUSYBOX.to_owned(),
        _ => workspace::PS_CMD.to_owned(),
    };
    let (code, out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    if code != 0 && out.trim().is_empty() {
        let error = if err.trim().is_empty() {
            "ps недоступен".to_string()
        } else {
            err.trim().to_string()
        };
        return Ok(json!({ "ok": false, "error": error }));
    }
    Ok(match kind {
        platform::Kind::Windows => platform::win::parse_ps(&out),
        platform::Kind::BusyBox => platform::busybox::parse_ps(&out),
        _ => workspace::parse_ps(&out),
    })
}

#[tauri::command]
pub async fn workspace_kill(state: State<'_, AppState>, session_id: String, pid: u32) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    // `kill` в Windows нет: там процесс снимает PowerShell.
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = platform::kill_cmd(kind, pid)?;
    let (code, _out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    if code != 0 {
        let error = if err.trim().is_empty() {
            format!("kill завершился с кодом {code}")
        } else {
            err.trim().to_string()
        };
        actionlog::record_session(
            &session_id,
            "process.kill",
            json!({ "pid": pid }),
            &Err::<(), _>(&error),
        );
        return Ok(json!({ "ok": false, "error": error }));
    }
    actionlog::record_session(
        &session_id,
        "process.kill",
        json!({ "pid": pid }),
        &Ok::<(), String>(()),
    );
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn workspace_services(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = match kind {
        platform::Kind::Windows => platform::ps(platform::cmd::SERVICES_WINDOWS),
        platform::Kind::BusyBox => platform::cmd::SERVICES_BUSYBOX.to_owned(),
        _ => workspace::SERVICES_CMD.to_owned(),
    };
    let (code, out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(match kind {
        platform::Kind::Windows => platform::win::parse_services(&out),
        platform::Kind::BusyBox => platform::busybox::parse_services(&out),
        _ => workspace::parse_services(code, &out, &err),
    })
}

#[tauri::command]
pub async fn workspace_service_action(
    state: State<'_, AppState>,
    session_id: String,
    name: String,
    action: String,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    // Управлять службой каждая система умеет по-своему: systemctl, rc-service, PowerShell.
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = platform::service_cmd(kind, &name, &action)?;
    let (code, _out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    if code != 0 {
        let error = if err.trim().is_empty() {
            format!("Служба {name}: действие {action} вернуло код {code}")
        } else {
            err.trim().to_string()
        };
        actionlog::record_session(
            &session_id,
            "service.action",
            json!({ "name": name, "action": action }),
            &Err::<(), _>(&error),
        );
        return Ok(json!({ "ok": false, "error": error }));
    }
    actionlog::record_session(
        &session_id,
        "service.action",
        json!({ "name": name, "action": action }),
        &Ok::<(), String>(()),
    );
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn workspace_logs(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    // На Windows роль journalctl играет журнал событий, и читается он совсем иначе.
    if kind == platform::Kind::Windows {
        let (_c, out, err) = ssh::exec(
            &s.handle,
            &platform::ps(platform::cmd::LOGS_WINDOWS),
            Some(s.cancel.subscribe()),
        )
        .await?;
        if out.trim().is_empty() && !err.trim().is_empty() {
            return Ok(json!({ "ok": false, "error": err.trim() }));
        }
        return Ok(platform::win::parse_logs(&out));
    }
    let (_code, out, err) = ssh::exec(&s.handle, workspace::LOGS_CMD, Some(s.cancel.subscribe())).await?;
    let text = if out.trim().is_empty() { err } else { out };
    Ok(json!({ "ok": true, "text": text }))
}
