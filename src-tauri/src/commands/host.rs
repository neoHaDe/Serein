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
    let cmd = platform::kill_cmd(kind, pid);
    run_action(&s, &session_id, "process.kill", json!({ "pid": pid }), cmd, |code| {
        format!("kill завершился с кодом {code}")
    })
    .await
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
    sudo_password: Option<String>,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    // Управлять службой каждая система умеет по-своему: systemctl, rc-service, PowerShell.
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = platform::service_cmd(kind, &name, &action);
    let mut detail = json!({ "name": name, "action": action });
    // На Windows права у самой сессии, sudo там нет - и переводить нечего.
    let (Ok(cmd), false) = (&cmd, kind == platform::Kind::Windows) else {
        return run_action(&s, &session_id, "service.action", detail, cmd, |code| {
            format!("Служба {name}: действие {action} вернуло код {code}")
        })
        .await;
    };
    let (result, via_sudo, need_password) = service_step(&s, &name, cmd, sudo_password).await?;
    if via_sudo {
        detail["sudo"] = json!(true);
    }
    actionlog::record_session(&session_id, "service.action", detail, &result);
    Ok(match result {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e, "needSudo": need_password }),
    })
}

/// Действие над службой юникса: как есть, а при отказе в правах - через sudo.
///
/// Возвращает итог, был ли sudo и нужен ли пароль, чтобы попробовать ещё раз. Внешний `Err` -
/// обрыв связи. Пароль, если он есть, уходит на стандартный ввод `sudo -S`.
async fn service_step(
    s: &ssh::SshSession,
    name: &str,
    cmd: &str,
    sudo_password: Option<String>,
) -> Result<(Result<(), String>, bool, bool), String> {
    use platform::privileged as p;
    if let Some(password) = sudo_password.filter(|v| !v.is_empty()) {
        let (code, out, err) = ssh::exec_with_input(
            &s.handle,
            &p::sudo_with_password(cmd),
            &format!("{password}\n"),
            Some(s.cancel.subscribe()),
        )
        .await?;
        if code == 0 {
            return Ok((Ok(()), true, false));
        }
        let text = format!("{out}\n{err}");
        // Неверный пароль - повод спросить снова, а не закрыть форму.
        let again = text.contains("incorrect password");
        return Ok((Err(p::service_error(name, &text)), true, again));
    }

    let (code, _out, err) = ssh::exec(&s.handle, &p::c_locale(cmd), Some(s.cancel.subscribe())).await?;
    if code == 0 {
        return Ok((Ok(()), false, false));
    }
    if !p::denied(&err) {
        return Ok((Err(p::service_error(name, &err)), false, false));
    }
    let (code, _out, err) = ssh::exec(&s.handle, &p::sudo_nopass(cmd), Some(s.cancel.subscribe())).await?;
    Ok(match code {
        0 => (Ok(()), true, false),
        p::SUDO_NEEDS_PASSWORD => (
            Err(format!(
                "Чтобы управлять службой {name}, нужны права администратора - введите пароль sudo"
            )),
            false,
            true,
        ),
        p::SUDO_MISSING => (
            Err(format!(
                "Чтобы управлять службой {name}, нужны права администратора, а sudo на сервере нет. \
                 Подключитесь под root или разрешите это пользователю правилом polkit"
            )),
            false,
            false,
        ),
        _ => (Err(p::service_error(name, &err)), true, false),
    })
}

/// Действие над хостом с записью в журнал.
///
/// Отказ до сервера (pid 1, недопустимое имя службы) и неудача на сервере приходят в панель
/// одинаково - `{ok: false, error}` - и оба пишутся в журнал. Ошибку вызова панель не
/// показывает: строка осталась бы «занятой», а человек - без объяснения.
async fn run_action(
    s: &ssh::SshSession,
    session_id: &str,
    action: &str,
    detail: Value,
    cmd: Result<String, String>,
    fallback: impl FnOnce(i32) -> String,
) -> Result<Value, String> {
    let result = match cmd {
        Ok(cmd) => {
            let (code, _out, err) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
            ssh::exit_result(code, &err, || fallback(code))
        }
        Err(e) => Err(e),
    };
    actionlog::record_session(session_id, action, detail, &result);
    Ok(match result {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e }),
    })
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
