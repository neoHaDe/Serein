//! Сессии: открытие, ввод, размер окна, закрытие, владение, ключи узлов, журнал вывода, туннели.

use crate::{
    actionlog, knownhosts, metrics, policy, pty, serial, ssh, ssh_agent, store, telnet, term_out, termsize, AppState,
    Session,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

pub(crate) fn emit_connected(app: &AppHandle, id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(160)).await;
        let _ = app.emit("session-status", json!({ "id": id, "status": "connected" }));
    });
}

#[tauri::command]
pub fn session_open_local(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    p: Value,
) -> Result<String, String> {
    if policy::forbids_local_terminal() {
        return Err("Локальный терминал запрещён политикой администратора".into());
    }
    let size = termsize::for_open(&p);
    let cwd = p.get("cwd").and_then(|v| v.as_str()).map(|s| s.to_string());
    let pref = store::settings_get()
        .get("localShell")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .to_string();
    let shell = pty::resolve_shell(&pref);
    let id = uuid::Uuid::new_v4().to_string();
    let sess = pty::open_local(app.clone(), id.clone(), shell, cwd, size.cols, size.rows)?;
    crate::sync::lock(&state.sessions).insert(id.clone(), Session::Local(sess));
    // Владельцем становится окно, которое сессию открыло: закрыть её сможет только оно.
    state.owners.claim(&id, window.label());
    emit_connected(&app, id.clone());
    Ok(id)
}

/// Доступные COM-порты - для выпадающего списка в форме сервера.
#[tauri::command]
pub fn serial_ports() -> Vec<Value> {
    serial::list_ports()
}

/// Открывает сессию по COM-порту. `p.serial` - секция параметров линии из профиля,
/// либо разовые настройки, если пользователь открывает порт без сохранённого профиля.
#[tauri::command]
pub fn session_open_serial(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    p: Value,
) -> Result<String, String> {
    // Профиль сервера имеет приоритет: в нём настройки, которые пользователь сохранил.
    let cfg = match p.get("serverId").and_then(|v| v.as_str()) {
        Some(sid) => {
            let srv = store::servers_list()
                .into_iter()
                .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(sid))
                .ok_or("Сервер не найден")?;
            srv.get("serial").cloned().ok_or("У профиля нет настроек COM-порта")?
        }
        None => p.get("serial").cloned().ok_or("Не заданы настройки COM-порта")?,
    };

    let id = uuid::Uuid::new_v4().to_string();
    let sess = serial::open_serial(app.clone(), id.clone(), &cfg)?;
    crate::sync::lock(&state.sessions).insert(id.clone(), Session::Serial(sess));

    // Первой строкой показываем параметры линии: с COM-портом молчащий экран
    // неотличим от неверной скорости, и это первое, что надо проверить.
    let _ = app.emit(
        "session-data",
        json!({ "id": id, "data": format!("\x1b[90m[{}]\x1b[0m\r\n", serial::describe(&cfg)) }),
    );
    // Владельцем становится окно, которое сессию открыло: закрыть её сможет только оно.
    state.owners.claim(&id, window.label());
    emit_connected(&app, id.clone());
    Ok(id)
}

/// BREAK на линию - им сетевое железо переводят в recovery.
#[tauri::command]
pub fn serial_send_break(state: State<'_, AppState>, id: String) -> Result<(), String> {
    match crate::sync::lock(&state.sessions).get(&id) {
        Some(Session::Serial(p)) => p.send_break(),
        _ => Err("Это не сессия COM-порта".into()),
    }
}

#[tauri::command]
pub fn serial_set_signal(state: State<'_, AppState>, id: String, line: String, on: bool) -> Result<(), String> {
    match crate::sync::lock(&state.sessions).get(&id) {
        Some(Session::Serial(p)) => p.set_signal(&line, on),
        _ => Err("Это не сессия COM-порта".into()),
    }
}

/// Открывает telnet- или «сырую» TCP-сессию.
///
/// `p.serverId` - подключение по сохранённому профилю; без него берём `host`/`port`/`mode`
/// прямо из запроса (разовое подключение из палитры). `cols`/`rows` нужны сразу: telnet
/// сообщает размер окна в момент согласования, и без них сервер считает экран 80x24.
#[tauri::command]
pub fn session_open_tcp(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    p: Value,
) -> Result<String, String> {
    let profile = match p.get("serverId").and_then(|v| v.as_str()) {
        Some(sid) => store::servers_list()
            .into_iter()
            .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(sid))
            .ok_or("Сервер не найден")?,
        None => p.clone(),
    };

    let kind = profile
        .get("connection")
        .and_then(|v| v.as_str())
        .or_else(|| p.get("connection").and_then(|v| v.as_str()))
        .unwrap_or("telnet");
    let mode = match kind {
        "telnet" => telnet::Mode::Telnet,
        "raw" => telnet::Mode::Raw,
        other => return Err(format!("Это не TCP-подключение: {other}")),
    };
    // У сырого TCP осмысленного порта по умолчанию нет - консольные серверы слушают
    // кто на 2000, кто на 4001. Пусть пользователь укажет явно.
    let default_port = if mode == telnet::Mode::Telnet { 23 } else { 0 };
    let (host, port) = telnet::endpoint(&profile, default_port);
    policy::check_host(&host)?;
    if port == 0 {
        return Err("Укажите порт: у TCP-подключения нет значения по умолчанию".into());
    }

    let size = termsize::for_open(&p);
    let eol = profile.get("telnetEol").and_then(|v| v.as_str());

    let id = uuid::Uuid::new_v4().to_string();
    let sess = telnet::open_tcp(app.clone(), id.clone(), mode, &host, port, eol, size)?;
    crate::sync::lock(&state.sessions).insert(id.clone(), Session::Tcp(sess));

    // Как и у COM-порта: молчащий экран не отличить от неверного порта, поэтому
    // первой строкой пишем, куда именно подключились.
    let _ = app.emit(
        "session-data",
        json!({ "id": id, "data": format!("[90m[{}][0m
", telnet::describe(mode, &host, port)) }),
    );
    // Владельцем становится окно, которое сессию открыло: закрыть её сможет только оно.
    state.owners.claim(&id, window.label());
    emit_connected(&app, id.clone());
    Ok(id)
}

/// Управляющая команда telnet: BREAK, Interrupt Process, Are You There и соседи.
/// На сетевом железе это единственный способ прервать зависшую команду.
#[tauri::command]
pub fn telnet_command(state: State<'_, AppState>, id: String, name: String) -> Result<(), String> {
    match crate::sync::lock(&state.sessions).get(&id) {
        Some(Session::Tcp(t)) => t.send_command(&name),
        _ => Err("Это не telnet-сессия".into()),
    }
}

#[tauri::command]
pub async fn session_open_ssh(
    window: tauri::Window,
    app: AppHandle,
    state: State<'_, AppState>,
    p: Value,
) -> Result<String, crate::error::OpenError> {
    let server_id = p.get("serverId").and_then(|v| v.as_str()).ok_or("Не задан serverId")?;
    let chain = match crate::chain::resolve(server_id) {
        Ok(c) => c,
        Err(e) => {
            actionlog::record(Some(server_id), None, "ssh.connect", json!({}), Err(e.clone()));
            return Err(e.into());
        }
    };
    let size = termsize::for_open(&p);
    let (cols, rows) = (u32::from(size.cols), u32::from(size.rows));
    let id = uuid::Uuid::new_v4().to_string();
    let ki = state.ki.clone();
    let host_keys = state.host_keys.clone();

    let sess =
        // Фазу сбоя не схлопываем в строку: по ней фронтенд решает, повторять ли попытку.
        match ssh::connect_chain(app.clone(), id.clone(), chain, cols, rows, ki, host_keys).await {
            Ok(s) => s,
            Err(e) => {
                let e = crate::error::OpenError::from(e);
                actionlog::record(Some(server_id), None, "ssh.connect", json!({}), Err(e.message.clone()));
                return Err(e);
            }
        };
    let sess = Arc::new(sess);
    actionlog::bind(&id, server_id);
    actionlog::record(Some(server_id), Some(&id), "ssh.connect", json!({}), Ok(()));
    // Туннели профиля + команда на подключении. Туннели идут тем же путём, что и кнопка:
    // с проверкой политики и записью в журнал.
    let server = store::server_with_secrets(server_id);
    if let Some(srv) = &server {
        if let Some(tunnels) = srv.get("tunnels").and_then(|v| v.as_array()) {
            for t in tunnels {
                let _ = open_tunnel(app.clone(), &state, &sess, &id, t.clone()).await;
            }
        }
        if let Some(cmd) = srv
            .get("executeOnConnect")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            let _ = sess.tx.send(ssh::SshCmd::Write(format!("{cmd}\r").into_bytes()));
        }
    }
    // Сборщик метрик заводится вместе с сессией, а не с панелью обзора: история за час
    // должна быть и у сервера, на обзор которого ещё не смотрели.
    metrics::spawn(id.clone(), sess.handle.clone(), sess.cancel.subscribe());
    crate::sync::lock(&state.sessions).insert(id.clone(), Session::Ssh(sess));
    // Владельцем становится окно, которое сессию открыло: закрыть её сможет только оно.
    state.owners.claim(&id, window.label());
    emit_connected(&app, id.clone());
    Ok(id)
}

#[tauri::command]
pub fn session_write(state: State<'_, AppState>, id: String, data: String) {
    if let Some(s) = crate::sync::lock(&state.sessions).get(&id) {
        match s {
            Session::Local(l) => l.write(&data),
            Session::Serial(p) => p.write(&data),
            Session::Tcp(t) => t.write(&data),
            Session::Ssh(s) => {
                actionlog::terminal_input(&id, &data);
                let _ = s.tx.send(ssh::SshCmd::Write(data.into_bytes()));
            }
        }
    }
}

#[tauri::command]
pub fn session_resize(state: State<'_, AppState>, p: Value) {
    let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let Some(size) = termsize::for_resize(&p) else {
        return;
    };
    if let Some(s) = crate::sync::lock(&state.sessions).get(&id) {
        match s {
            Session::Local(l) => l.resize(size.cols, size.rows),
            // У последовательного порта нет размера окна - ресайз игнорируем.
            Session::Serial(_) => {}
            // У telnet размер окна есть (NAWS); у сырого TCP отправка молча пропускается.
            Session::Tcp(t) => t.resize(size.cols, size.rows),
            Session::Ssh(s) => {
                let _ =
                    s.tx.send(ssh::SshCmd::Resize(u32::from(size.cols), u32::from(size.rows)));
            }
        }
    }
}

/// Закрыть сессию. Разрешено только окну-владельцу.
///
/// Проверка здесь, а не во фронтенде, намеренно. Раньше каждое окно само решало, «его» ли
/// это сессия, по своему набору пометок - и откреплённое окно, не знавшее о пометках
/// главного, закрывало чужую сессию при первом переключении на Docker. Теперь запрос от
/// не-владельца просто игнорируется: окну незачем знать чужую бухгалтерию.
#[tauri::command]
pub fn session_close(window: tauri::Window, app: AppHandle, state: State<'_, AppState>, id: String) {
    if !state.owners.may_close(&id, window.label()) {
        return;
    }
    state.teardown(&app, &id, true);
}

/// Передать владение сессией окну с указанной меткой.
///
/// Зовётся при откреплении (главное окно отдаёт новому) и при возврате вкладки
/// (откреплённое отдаёт главному) - до того, как прежнее окно начнёт разбираться.
#[tauri::command]
pub fn session_claim(state: State<'_, AppState>, id: String, window_label: String) {
    state.owners.claim(&id, &window_label);
}

/// Хвост вывода сессии.
///
/// Нужен, когда вкладка переезжает между окнами: сессия та же, а xterm новый и пустой -
/// шелл ничего не перерисует, пока не нажмёшь Enter. Отдаём то, что уже было на экране.
#[tauri::command]
pub fn session_replay(id: String) -> String {
    term_out::replay(&id)
}

#[tauri::command]
pub async fn session_ping(state: State<'_, AppState>, id: String) -> Result<Option<u32>, String> {
    match state.ssh(&id) {
        Some(s) => Ok(ssh::ping(&s.handle).await),
        None => Ok(None),
    }
}

/// Ответ пользователя на вопрос о ключе хоста (доверять или нет).
#[tauri::command]
pub fn session_hostkey_respond(state: State<'_, AppState>, request_id: String, accept: bool) {
    if let Some(tx) = crate::sync::lock(&state.host_keys).remove(&request_id) {
        let _ = tx.send(accept);
    }
}

/// Известные ключи хостов - список для настроек.
#[tauri::command]
pub fn knownhosts_list() -> Vec<Value> {
    knownhosts::list()
}

/// Забыть хост: при следующем подключении ключ спросят заново.
#[tauri::command]
pub fn knownhosts_forget(host: String) -> bool {
    knownhosts::forget(&host)
}

/// Импорт отпечатков из `~/.ssh/known_hosts` - чтобы не подтверждать заново то,
/// чему пользователь уже доверился в OpenSSH.
#[tauri::command]
pub fn knownhosts_import() -> Result<Value, String> {
    let added = knownhosts::import_openssh()?;
    Ok(json!({ "imported": added }))
}

/// Ключи локального SSH-агента - для выбора в настройках сервера.
#[tauri::command]
pub async fn ssh_agent_identities() -> Result<Value, String> {
    match ssh_agent::list_identities().await {
        Ok(keys) => Ok(json!({
            "ok": true,
            "keys": keys.iter().map(|k| k.to_json()).collect::<Vec<_>>(),
        })),
        // Отсутствие агента - обычное состояние, а не сбой: форма покажет подсказку.
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

#[tauri::command]
pub fn session_log_status(id: String) -> bool {
    term_out::log_active(&id)
}

/// Включает или выключает запись вывода сессии в файл (`%APPDATA%\serein\logs`).
#[tauri::command]
pub fn session_log_toggle(id: String, title: String) -> Result<Value, String> {
    if term_out::log_active(&id) {
        let path = term_out::log_path_of(&id);
        term_out::log_stop(&id);
        return Ok(json!({ "logging": false, "path": path }));
    }
    if policy::forbids_session_recording() {
        return Err("Запись сессии в файл запрещена политикой администратора".into());
    }
    let path = term_out::log_start(&id, &title)?;
    Ok(json!({ "logging": true, "path": path }))
}

#[tauri::command]
pub fn session_ki_respond(state: State<'_, AppState>, id: String, answers: Vec<String>) {
    if let Some(tx) = crate::sync::lock(&state.ki).remove(&id) {
        let _ = tx.send(answers);
    }
}

#[tauri::command]
pub fn tunnel_list_status(state: State<'_, AppState>, session_id: String) -> Vec<Value> {
    state.tunnels.list_status(&session_id)
}

#[tauri::command]
pub async fn tunnel_open(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    tunnel_id: String,
) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let server = store::server_with_secrets(&s.server_id).ok_or("Сервер не найден")?;
    let cfg = server
        .get("tunnels")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(tunnel_id.as_str()))
        })
        .cloned()
        .ok_or("Конфиг туннеля не найден")?;
    open_tunnel(app, &state, &s, &session_id, cfg).await
}

/// Открывает туннель и пишет итог в журнал. Один путь и для кнопки, и для туннелей, которые
/// поднимаются при подключении: раньше вторые шли мимо журнала. Тип и политику проверяет
/// сам `TunnelManager::open`.
async fn open_tunnel(
    app: AppHandle,
    state: &AppState,
    sess: &ssh::SshSession,
    session_id: &str,
    cfg: Value,
) -> Result<(), String> {
    let detail = json!({
        "tunnel": cfg.get("id"),
        "type": cfg.get("type"),
        "localPort": cfg.get("localPort"),
        "remoteHost": cfg.get("remoteHost"),
        "remotePort": cfg.get("remotePort"),
    });
    let r = state
        .tunnels
        .open(
            app,
            sess.handle.clone(),
            session_id.to_owned(),
            cfg,
            sess.remote_forwards.clone(),
            sess.cancel.subscribe(),
        )
        .await;
    actionlog::record_session(session_id, "tunnel.open", detail, &r);
    r
}

#[tauri::command]
pub fn tunnel_close(app: AppHandle, state: State<'_, AppState>, session_id: String, tunnel_id: String) {
    state.tunnels.close(&session_id, &tunnel_id, &app);
}
