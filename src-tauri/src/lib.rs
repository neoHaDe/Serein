//! Serein - backend. Команды Tauri и менеджер сессий.

mod backup;
mod clipboard;
mod crypto;
mod dnd;
mod docker;
mod docker_compose;
mod dpapi;
mod error;
pub mod filediff;
mod importers;
mod os_secrets;
mod ownership;
mod keygen;
mod knownhosts;
pub mod ldap;
mod localfs;
pub mod monitor;
mod multihost;
pub mod mysql;
mod paths;
pub mod sysinfo;
mod proxycmd;
mod schema;
mod pty;
mod remoteedit;
pub mod remote_fs;
pub mod db;
pub mod platform;
pub mod scp;
pub mod rdp;
pub mod vnc;
pub mod vncsetup;
pub mod rdpsetup;
pub mod deskout;
mod serial;
pub mod sftp;
mod ssh_agent;
mod ssh_algos;
pub mod ssh;
pub mod store;
mod sync;
mod telnet;
mod term_out;
pub mod tools;
mod tunnels;
mod vault;
mod vaultkey;
pub mod workspace;

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

pub(crate) enum Session {
    Local(pty::LocalSession),
    Ssh(Arc<ssh::SshSession>),
    Serial(serial::SerialSession),
    /// Telnet или «сырой» TCP - общий транспорт, разный разбор потока.
    Tcp(telnet::TcpSession),
}

pub(crate) struct AppState {
    sessions: Mutex<HashMap<String, Session>>,
    ki: ssh::KiBridge,
    host_keys: ssh::HostKeyBridge,
    tunnels: tunnels::TunnelManager,
    edit: remoteedit::EditManager,
    transfers: sftp::TransferHub,
    ops: ssh::OpHub,
    /// Какому окну принадлежит сессия. Закрыть её может только владелец - см. `ownership`.
    owners: ownership::Owners,
}

impl AppState {
    fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            ki: Arc::new(Mutex::new(HashMap::new())),
            host_keys: Arc::new(Mutex::new(HashMap::new())),
            tunnels: tunnels::TunnelManager::default(),
            edit: remoteedit::EditManager::default(),
            transfers: sftp::TransferHub::default(),
            ops: ssh::OpHub::default(),
            owners: ownership::Owners::default(),
        }
    }
    fn ssh(&self, id: &str) -> Option<Arc<ssh::SshSession>> {
        match crate::sync::lock(&self.sessions).get(id) {
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
        term_out::replay_forget(id);
        platform::forget(id);
        sysinfo_forget(id);
        db::close_session(id);
        self.owners.release(id);
        if let Some(tx) = crate::sync::lock(&self.ki).remove(id) {
            drop(tx);
        }
        if let Some(s) = crate::sync::lock(&self.sessions).remove(id) {
            match s {
                Session::Local(l) => l.close(),
                Session::Serial(p) => p.close(),
                Session::Tcp(t) => t.close(),
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
fn app_platform() -> &'static str {
    #[cfg(windows)]
    {
        "windows"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        "other"
    }
}

/// Куда приложение на самом деле пишет профиль и логи сессий.
///
/// Не украшение: на Linux запуск из меню приложений и из терминала может прийти с разным
/// `HOME`/`XDG_CONFIG_HOME`, и тогда приложение молча открывает пустой профиль - список
/// серверов выглядит потерянным. По этой строке разница видна за секунду.
#[tauri::command]
fn app_paths() -> Value {
    let cfg = store::config_dir();
    json!({
        "config": cfg.to_string_lossy(),
        "logs": cfg.join("logs").to_string_lossy(),
    })
}

/// Как приложение установлено - от этого зависит, можно ли обновиться на месте.
///
/// `installer` - Windows: апдейтер скачивает установщик и перезапускает приложение.
/// `appimage` - Linux, запуск из AppImage: файл заменяется целиком, это единственная
/// форма на Linux, которую умеет обновлять сам Tauri (переменную `APPIMAGE` выставляет
/// среда выполнения AppImage).
/// `package` - Linux из `.deb`: бинарь лежит в `/usr/bin` и принадлежит менеджеру пакетов,
/// писать туда приложение не может и не должно. Обновление - через пакет.
#[tauri::command]
fn app_install_kind() -> &'static str {
    #[cfg(windows)]
    {
        "installer"
    }
    #[cfg(not(windows))]
    {
        if std::env::var_os("APPIMAGE").is_some() {
            "appimage"
        } else {
            "package"
        }
    }
}

/// Одна команда на нескольких серверах. Результат каждого хоста уходит событием
/// сразу, как только готов; здесь возвращается общий список - для истории в окне.
#[tauri::command]
async fn multi_exec(
    app: AppHandle,
    state: State<'_, AppState>,
    server_ids: Vec<String>,
    command: String,
) -> Result<Vec<Value>, String> {
    let cancel = state.ops.begin("multi-exec");
    let out = multihost::run(app, server_ids, command, cancel).await;
    state.ops.finish("multi-exec");
    Ok(out)
}

#[tauri::command]
fn multi_exec_cancel(state: State<'_, AppState>) {
    state.ops.cancel("multi-exec");
}

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
/// Перестановка серверов после перетаскивания: `[{ id, group, order }]`.
#[tauri::command]
fn servers_reorder(items: Vec<Value>) -> Result<(), String> {
    store::servers_reorder(&items)
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

/// Цепочка «целевой сервер → jump-хосты» с расшифрованными секретами.
pub(crate) fn resolve_chain_for(server_id: &str) -> Result<Vec<Value>, String> {
    resolve_chain(server_id)
}

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
fn session_open_local(window: tauri::Window, app: AppHandle, state: State<'_, AppState>, p: Value) -> Result<String, String> {
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
    crate::sync::lock(&state.sessions).insert(id.clone(), Session::Local(sess));
    // Владельцем становится окно, которое сессию открыло: закрыть её сможет только оно.
    state.owners.claim(&id, window.label());
    emit_connected(&app, id.clone());
    Ok(id)
}

/// Доступные COM-порты - для выпадающего списка в форме сервера.
#[tauri::command]
fn serial_ports() -> Vec<Value> {
    serial::list_ports()
}

/// Открывает сессию по COM-порту. `p.serial` - секция параметров линии из профиля,
/// либо разовые настройки, если пользователь открывает порт без сохранённого профиля.
#[tauri::command]
fn session_open_serial(window: tauri::Window, app: AppHandle, state: State<'_, AppState>, p: Value) -> Result<String, String> {
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
fn serial_send_break(state: State<'_, AppState>, id: String) -> Result<(), String> {
    match crate::sync::lock(&state.sessions).get(&id) {
        Some(Session::Serial(p)) => p.send_break(),
        _ => Err("Это не сессия COM-порта".into()),
    }
}

#[tauri::command]
fn serial_set_signal(state: State<'_, AppState>, id: String, line: String, on: bool) -> Result<(), String> {
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
fn session_open_tcp(window: tauri::Window, app: AppHandle, state: State<'_, AppState>, p: Value) -> Result<String, String> {
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
    if port == 0 {
        return Err("Укажите порт: у TCP-подключения нет значения по умолчанию".into());
    }

    let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(80).clamp(20, 500) as u16;
    let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(24).clamp(5, 200) as u16;
    let eol = profile.get("telnetEol").and_then(|v| v.as_str());

    let id = uuid::Uuid::new_v4().to_string();
    let sess = telnet::open_tcp(app.clone(), id.clone(), mode, &host, port, eol, cols, rows)?;
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
fn telnet_command(state: State<'_, AppState>, id: String, name: String) -> Result<(), String> {
    match crate::sync::lock(&state.sessions).get(&id) {
        Some(Session::Tcp(t)) => t.send_command(&name),
        _ => Err("Это не telnet-сессия".into()),
    }
}

#[tauri::command]
async fn session_open_ssh(window: tauri::Window, app: AppHandle, state: State<'_, AppState>, p: Value) -> Result<String, crate::error::OpenError> {
    let server_id = p.get("serverId").and_then(|v| v.as_str()).ok_or("Не задан serverId")?;
    let chain = resolve_chain(server_id)?;
    let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u32;
    let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u32;
    let id = uuid::Uuid::new_v4().to_string();
    let ki = state.ki.clone();
    let host_keys = state.host_keys.clone();

    let sess =
        // Фазу сбоя не схлопываем в строку: по ней фронтенд решает, повторять ли попытку.
        ssh::connect_chain(app.clone(), id.clone(), chain, cols, rows, ki, host_keys).await?;
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
    crate::sync::lock(&state.sessions).insert(id.clone(), Session::Ssh(sess));
    // Владельцем становится окно, которое сессию открыло: закрыть её сможет только оно.
    state.owners.claim(&id, window.label());
    emit_connected(&app, id.clone());
    Ok(id)
}

#[tauri::command]
fn session_write(state: State<'_, AppState>, id: String, data: String) {
    if let Some(s) = crate::sync::lock(&state.sessions).get(&id) {
        match s {
            Session::Local(l) => l.write(&data),
            Session::Serial(p) => p.write(&data),
            Session::Tcp(t) => t.write(&data),
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
    if let Some(s) = crate::sync::lock(&state.sessions).get(&id) {
        match s {
            Session::Local(l) => l.resize(cols as u16, rows as u16),
            // У последовательного порта нет размера окна - ресайз игнорируем.
            Session::Serial(_) => {}
            // У telnet размер окна есть (NAWS); у сырого TCP отправка молча пропускается.
            Session::Tcp(t) => t.resize(cols as u16, rows as u16),
            Session::Ssh(s) => {
                let _ = s.tx.send(ssh::SshCmd::Resize(cols as u32, rows as u32));
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
fn session_close(window: tauri::Window, app: AppHandle, state: State<'_, AppState>, id: String) {
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
fn session_claim(state: State<'_, AppState>, id: String, window_label: String) {
    state.owners.claim(&id, &window_label);
}

/// Хвост вывода сессии.
///
/// Нужен, когда вкладка переезжает между окнами: сессия та же, а xterm новый и пустой -
/// шелл ничего не перерисует, пока не нажмёшь Enter. Отдаём то, что уже было на экране.
#[tauri::command]
fn session_replay(id: String) -> String {
    term_out::replay(&id)
}

#[tauri::command]
async fn session_ping(state: State<'_, AppState>, id: String) -> Result<Option<u32>, String> {
    match state.ssh(&id) {
        Some(s) => Ok(ssh::ping(&s.handle).await),
        None => Ok(None),
    }
}

/// Ответ пользователя на вопрос о ключе хоста (доверять или нет).
#[tauri::command]
fn session_hostkey_respond(state: State<'_, AppState>, request_id: String, accept: bool) {
    if let Some(tx) = crate::sync::lock(&state.host_keys).remove(&request_id) {
        let _ = tx.send(accept);
    }
}

/// Известные ключи хостов - список для настроек.
#[tauri::command]
fn knownhosts_list() -> Vec<Value> {
    knownhosts::list()
}

/// Забыть хост: при следующем подключении ключ спросят заново.
#[tauri::command]
fn knownhosts_forget(host: String) -> bool {
    knownhosts::forget(&host)
}

/// Импорт отпечатков из `~/.ssh/known_hosts` - чтобы не подтверждать заново то,
/// чему пользователь уже доверился в OpenSSH.
#[tauri::command]
fn knownhosts_import() -> Result<Value, String> {
    let added = knownhosts::import_openssh()?;
    Ok(json!({ "imported": added }))
}

/// Ключи локального SSH-агента - для выбора в настройках сервера.
#[tauri::command]
async fn ssh_agent_identities() -> Result<Value, String> {
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

/// Железо сервера: процессор, видео, память, виртуализация.
///
/// Собирается один раз за сессию и запоминается: модель процессора не меняется, а панель
/// обзора обновляется каждые несколько секунд - спрашивать это по таймеру значило бы
/// впустую гонять канал.
#[tauri::command]
async fn session_sysinfo(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    if let Some(v) = sysinfo_cached(&id) {
        return Ok(v);
    }
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&id, &s.handle).await;
    if kind == platform::Kind::Windows {
        return Ok(json!({ "gpus": [], "unsupported": "Сведения о железе Windows-сервера пока не читаем" }));
    }
    let (_c, out, _e) = ssh::exec(&s.handle, sysinfo::CMD, Some(s.cancel.subscribe())).await?;
    let v = sysinfo::parse(&out);
    crate::sync::lock(&SYSINFO)
        .get_or_insert_with(Default::default)
        .insert(id, v.clone());
    Ok(v)
}

/// Запомненные сведения о железе. Чистятся вместе с сессией.
static SYSINFO: std::sync::Mutex<Option<std::collections::HashMap<String, Value>>> =
    std::sync::Mutex::new(None);

fn sysinfo_cached(id: &str) -> Option<Value> {
    crate::sync::lock(&SYSINFO)
        .get_or_insert_with(Default::default)
        .get(id)
        .cloned()
}

fn sysinfo_forget(id: &str) {
    crate::sync::lock(&SYSINFO)
        .get_or_insert_with(Default::default)
        .remove(id);
}

#[tauri::command]
async fn session_monitor(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    // Снимок собирается одной командой, но команда у Windows своя: /proc там нет.
    let (kind, _) = platform::of_session(&id, &s.handle).await;
    if kind == platform::Kind::Windows {
        let (_c, out, err) =
            ssh::exec(&s.handle, platform::cmd::SAMPLE_WINDOWS, Some(s.cancel.subscribe())).await?;
        if out.trim().is_empty() && !err.trim().is_empty() {
            return Ok(json!({ "ok": false, "error": err.trim() }));
        }
        return Ok(platform::win::parse_sample(&out));
    }
    let (_c, out, _e) = ssh::exec(&s.handle, monitor::SAMPLE_CMD, Some(s.cancel.subscribe())).await?;
    Ok(monitor::parse(&out))
}

/// Подключается к базе данных рядом с сервером.
///
/// Через ту же SSH-сессию, а не отдельным соединением: база слушает петлю сервера и в сеть
/// не смотрит. Иначе пришлось бы вручную поднимать проброс порта и помнить, что он открыт.
#[tauri::command]
async fn db_open(
    state: State<'_, AppState>,
    session_id: String,
    params: db::Params,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let id = format!("db-{}", uuid::Uuid::new_v4());
    db::open(id, &session_id, &s.handle, params).await
}

/// Выполняет запрос: SQL для PostgreSQL, команду для Redis.
#[tauri::command]
async fn db_query(id: String, text: String) -> Result<Value, String> {
    db::query(&id, &text).await
}

/// Закрытие идёт асинхронной командой не для красоты: деструктор SSH-канала обращается
/// к рантайму Tokio, и с главного потока это роняло всё приложение целиком.
#[tauri::command]
async fn db_close(id: String) {
    db::close(&id);
}

/// Что за база уже открыта в этой сессии, если открыта.
///
/// Панель спрашивает об этом, когда не помнит ничего сама: в откреплённом окне своя
/// память, а соединение общее и живёт в приложении.
#[tauri::command]
async fn db_current(session_id: String) -> Option<Value> {
    db::for_session(&session_id)
}

/// Что на сервере есть для рабочего стола: программа, порты, среда, права.
///
/// Спрашивается перед подключением, а не после неудачи. Раньше про отсутствие VNC можно
/// было узнать только по невнятной ошибке соединения - а это три разные беды с тремя
/// разными ответами: программы нет, программа не запущена, запущена не там.
#[tauri::command]
async fn desktop_detect(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    if kind == platform::Kind::Windows {
        return Ok(json!({
            "installed": [], "listening": [], "canInstall": false,
            "summary": "На Windows-сервере рабочий стол настраивается иначе - этого мы пока не умеем",
        }));
    }
    let (_c, out, _e) =
        ssh::exec(&s.handle, vncsetup::DETECT_CMD, Some(s.cancel.subscribe())).await?;
    Ok(vncsetup::parse_detect(&out))
}

/// Ставит сервер VNC на сервер.
///
/// Пароль `sudo` уходит на стандартный ввод, а не в строку команды: она целиком видна в
/// списке процессов сервера любому, кто там есть. Нигде не сохраняется.
#[tauri::command]
async fn desktop_install(
    state: State<'_, AppState>,
    session_id: String,
    package_manager: String,
    sudo_password: String,
) -> Result<Value, String> {
    let cmd = vncsetup::install_cmd(&package_manager)
        .ok_or("Не знаем, как ставить пакеты этим менеджером")?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (code, out, err) =
        ssh::exec_with_input(&s.handle, &cmd, &format!("{sudo_password}\n"), Some(s.cancel.subscribe()))
            .await?;
    if code != 0 {
        // Неверный пароль sudo выглядит именно так, и сказать об этом прямо полезнее,
        // чем показать сырой вывод пакетного менеджера.
        let текст = if out.contains("incorrect password") || err.contains("incorrect password") {
            "Пароль sudo не подошёл".to_string()
        } else {
            let x = format!("{out}\n{err}");
            let x = x.trim();
            if x.is_empty() { format!("установка вернула код {code}") } else { x.to_string() }
        };
        return Ok(json!({ "ok": false, "error": текст }));
    }
    Ok(json!({ "ok": true }))
}

/// Задаёт пароль рабочего стола.
///
/// Выполняется от самого пользователя, без sudo: файл пароля лежит в его домашнем каталоге.
#[tauri::command]
async fn desktop_set_password(
    state: State<'_, AppState>,
    session_id: String,
    password: String,
) -> Result<Value, String> {
    vncsetup::check_vnc_password(&password)?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (code, out, err) = ssh::exec_with_input(
        &s.handle,
        vncsetup::SET_PASSWORD_CMD,
        &format!("{password}\n"),
        Some(s.cancel.subscribe()),
    )
    .await?;
    if code != 0 || !out.contains("OK") {
        // Сюда попадает то, что сказала сама программа. Раньше её вывод глушился, и панель
        // показывала «не удалось сохранить пароль» без единого слова о причине - на сервере
        // с tigervnc это выглядело как поломка на пустом месте.
        let x = format!("{err}\n{out}");
        let x = x.trim();
        return Ok(json!({
            "ok": false,
            "error": if x.is_empty() { format!("команда вернула код {code}") } else { x.to_string() },
        }));
    }
    Ok(json!({ "ok": true }))
}

/// Открывает рабочий стол VNC поверх уже подключённой SSH-сессии.
///
/// Через сессию, а не напрямую, потому что VNC на сервере почти всегда слушает `127.0.0.1`
/// и наружу не смотрит - и правильно делает: свой протокол он защищает паролем до восьми
/// символов на DES. Ходить к нему нужно внутри SSH, а не открывать порт в сеть.
#[tauri::command]
async fn vnc_open(
    state: State<'_, AppState>,
    session_id: String,
    host: Option<String>,
    port: Option<u16>,
    password: Option<String>,
    on_frame: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<String, vnc::OpenError> {
    let s = state
        .ssh(&session_id)
        .ok_or_else(|| vnc::OpenError::from("Сессия не подключена".to_string()))?;
    let id = format!("vnc-{}", uuid::Uuid::new_v4());
    let target = vnc::Target::Ssh {
        handle: s.handle.clone(),
        host: host.unwrap_or_else(|| "127.0.0.1".into()),
        // 5900 - нулевой дисплей; у большинства серверов рабочий стол именно там.
        port: port.unwrap_or(5900),
    };
    vnc::open(id.clone(), session_id.clone(), target, password, on_frame).await?;
    Ok(id)
}

/// Движение мыши и нажатия. Кнопки - битовой маской, как в RFB: 1 левая, 2 средняя,
/// 4 правая, 8 и 16 - колесо вверх и вниз.
#[tauri::command]
fn vnc_pointer(id: String, x: u16, y: u16, buttons: u8) {
    vnc::input(&id, vnc::X11Event::PointerEvent((x, y, buttons).into()));
}

#[tauri::command]
fn vnc_key(id: String, keysym: u32, down: bool) {
    vnc::input(&id, vnc::X11Event::KeyEvent((keysym, down).into()));
}

/// Запрос обновления экрана. Полное обновление нужно после переподключения или когда
/// картинка «поехала»: сервер шлёт только изменения и сам себя не перерисовывает.
#[tauri::command]
fn vnc_refresh(id: String, full: bool) {
    vnc::input(
        &id,
        if full { vnc::X11Event::FullRefresh } else { vnc::X11Event::Refresh },
    );
}

/// Вставка на удалённый рабочий стол.
///
/// В RFB буфер обмена и вставка - разные вещи: `ClientCutText` только кладёт текст в буфер
/// сервера, но никуда его не вставляет. Поэтому следом отправляется Shift+Insert - это
/// сочетание понимают и xterm, и обычные приложения X, в отличие от Ctrl+V, который в
/// терминалах не работает.
#[tauri::command]
fn vnc_paste(id: String, text: String) {
    vnc::paste(&id, text);
}

#[tauri::command]
fn vnc_close(id: String) {
    vnc::close(&id);
}

/// Есть ли у этой SSH-сессии уже открытый рабочий стол.
///
/// Спрашивается при открытии панели. Нужно для откреплённого окна: сеанс живёт в
/// приложении, а не в окне, и второе окно должно продолжить картинку, а не начинать с
/// ввода пароля - тот же сеанс, тот же сервер, зачем спрашивать дважды.
#[tauri::command]
fn desktop_active(session_id: String) -> Option<Value> {
    deskout::active(&session_id).map(|a| {
        json!({ "kind": a.kind.as_str(), "id": a.id, "width": a.size.0, "height": a.size.1 })
    })
}

/// Переводит выдачу кадров VNC в это окно и просит перерисовать экран целиком.
#[tauri::command]
fn vnc_attach(id: String, on_frame: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>) -> Result<(), String> {
    vnc::attach(&id, on_frame)
}

/// То же для RDP.
#[tauri::command]
fn rdp_attach(id: String, on_frame: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>) -> Result<(), String> {
    rdp::attach(&id, on_frame)
}

/// Что на сервере есть для RDP: программа, порт, служба, права.
///
/// Отдельно от разведки VNC: смотреть надо другое, а лишний вопрос серверу дешевле, чем
/// одна команда, отвечающая сразу за двоих и путающая оба ответа.
#[tauri::command]
async fn desktop_rdp_detect(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    if kind == platform::Kind::Windows {
        // На Windows RDP свой, встроенный, и ставить нечего - но он может быть выключен.
        return Ok(json!({
            "installed": [], "listening": [], "canInstall": false, "canStart": false,
            "summary": "На Windows рабочий стол включается в настройках системы, ставить нечего",
        }));
    }
    let (_c, out, _e) =
        ssh::exec(&s.handle, rdpsetup::DETECT_CMD, Some(s.cancel.subscribe())).await?;
    Ok(rdpsetup::parse_detect(&out))
}

/// Ставит xrdp на сервер.
#[tauri::command]
async fn desktop_rdp_install(
    state: State<'_, AppState>,
    session_id: String,
    package_manager: String,
    sudo_password: String,
) -> Result<Value, String> {
    let cmd = rdpsetup::install_cmd(&package_manager)
        .ok_or("Этим менеджером пакетов xrdp не поставить: пакета нет в основных хранилищах")?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    run_setup_step(&s, &cmd, &sudo_password, "установка").await
}

/// Включает и запускает службу xrdp.
///
/// Именно включает, а не только запускает: иначе после перезагрузки сервера рабочий стол
/// молча не поднимется, и выяснится это в самый неудачный момент.
#[tauri::command]
async fn desktop_rdp_start(
    state: State<'_, AppState>,
    session_id: String,
    sudo_password: String,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let r = run_setup_step(&s, rdpsetup::ENABLE_CMD, &sudo_password, "запуск").await?;
    if r["ok"] != true {
        return Ok(r);
    }
    // `systemctl` возвращает ноль, успев только отправить запрос. Служба, упавшая
    // секундой позже, ответила бы «готово» - поэтому спрашиваем её саму.
    let out = r["output"].as_str().unwrap_or_default();
    if out.lines().any(|l| l.trim() == "active") {
        Ok(json!({ "ok": true }))
    } else {
        Ok(json!({
            "ok": false,
            "error": format!("служба не поднялась: {}", out.trim()),
        }))
    }
}

/// Общая часть установки и запуска: выполнить с паролем на входе и разобрать отказ.
///
/// Неверный пароль sudo выглядит одинаково в обоих случаях, и сказать об этом прямо
/// полезнее, чем показать сырой вывод команды.
async fn run_setup_step(
    s: &std::sync::Arc<ssh::SshSession>,
    cmd: &str,
    sudo_password: &str,
    что: &str,
) -> Result<Value, String> {
    let (code, out, err) = ssh::exec_with_input(
        &s.handle,
        cmd,
        &format!("{sudo_password}\n"),
        Some(s.cancel.subscribe()),
    )
    .await?;
    if code != 0 {
        let текст = if out.contains("incorrect password") || err.contains("incorrect password") {
            "Пароль sudo не подошёл".to_string()
        } else {
            let x = format!("{out}\n{err}");
            let x = x.trim();
            if x.is_empty() { format!("{что} вернулась с кодом {code}") } else { x.to_string() }
        };
        return Ok(json!({ "ok": false, "error": текст }));
    }
    Ok(json!({ "ok": true, "output": out }))
}

/// Открывает рабочий стол по RDP.
///
/// Протокол разбирает отдельный процесс, а не этот модуль: зависимости IronRDP не
/// сходятся с SSH-ядром в одном дереве, подробности в `rdp.rs`. Приложение здесь держит
/// канал внутри SSH-сессии и подставляет его помощнику локальным сокетом.
#[tauri::command]
async fn rdp_open(
    state: State<'_, AppState>,
    session_id: String,
    host: Option<String>,
    port: Option<u16>,
    user: String,
    password: String,
    domain: Option<String>,
    width: Option<u16>,
    height: Option<u16>,
    color_depth: Option<u16>,
    economy: Option<bool>,
    autologon: Option<bool>,
    on_frame: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<String, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let id = format!("rdp-{}", uuid::Uuid::new_v4());
    let target = rdp::Target::Ssh {
        handle: s.handle.clone(),
        host: host.unwrap_or_else(|| "127.0.0.1".to_owned()),
        port: port.unwrap_or(3389),
    };
    rdp::open(
        id.clone(),
        session_id.clone(),
        target,
        user,
        password,
        domain,
        (width.unwrap_or(1280), height.unwrap_or(800)),
        rdp::Options {
            color_depth: color_depth.unwrap_or(32),
            economy: economy.unwrap_or(false),
            autologon: autologon.unwrap_or(true),
        },
        on_frame,
    )
    .await?;
    Ok(id)
}

#[tauri::command]
fn rdp_pointer(id: String, x: u16, y: u16, buttons: u8) {
    rdp::pointer(&id, x, y, buttons);
}

#[tauri::command]
fn rdp_key(id: String, code: u16, down: bool) {
    rdp::key(&id, code, down);
}

/// Меняет размер рабочего стола в уже открытом сеансе.
#[tauri::command]
fn rdp_resize(id: String, width: u16, height: u16) {
    rdp::resize(&id, width, height);
}

#[tauri::command]
fn rdp_close(id: String) {
    rdp::close(&id);
}

/// Отчёт интерфейса о своей половине пути кадра.
#[tauri::command]
fn rdp_note(line: String) {
    rdp::note(&line);
}

/// Какая система на сервере. Определяется один раз за сессию и кэшируется.
#[tauri::command]
async fn workspace_platform(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, version) = platform::of_session(&session_id, &s.handle).await;
    Ok(platform::to_json(kind, &version))
}

#[tauri::command]
async fn workspace_processes(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    // Набор команд зависит от системы: `ps` на Windows не существует, и слать его туда
    // значит показать пользователю ошибку вместо таблицы процессов.
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = match kind {
        platform::Kind::Windows => platform::cmd::PS_WINDOWS,
        platform::Kind::BusyBox => platform::cmd::PS_BUSYBOX,
        _ => workspace::PS_CMD,
    };
    let (code, out, err) = ssh::exec(&s.handle, cmd, Some(s.cancel.subscribe())).await?;
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
async fn workspace_kill(state: State<'_, AppState>, session_id: String, pid: u32) -> Result<Value, String> {
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
        return Ok(json!({ "ok": false, "error": error }));
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
async fn workspace_services(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = match kind {
        platform::Kind::Windows => platform::cmd::SERVICES_WINDOWS,
        platform::Kind::BusyBox => platform::cmd::SERVICES_BUSYBOX,
        _ => workspace::SERVICES_CMD,
    };
    let (code, out, err) = ssh::exec(&s.handle, cmd, Some(s.cancel.subscribe())).await?;
    Ok(match kind {
        platform::Kind::Windows => platform::win::parse_services(&out),
        platform::Kind::BusyBox => platform::busybox::parse_services(&out),
        _ => workspace::parse_services(code, &out, &err),
    })
}

#[tauri::command]
async fn workspace_service_action(
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
        return Ok(json!({ "ok": false, "error": error }));
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
async fn workspace_logs(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    // На Windows роль journalctl играет журнал событий, и читается он совсем иначе.
    if kind == platform::Kind::Windows {
        let (_c, out, err) =
            ssh::exec(&s.handle, platform::cmd::LOGS_WINDOWS, Some(s.cancel.subscribe())).await?;
        if out.trim().is_empty() && !err.trim().is_empty() {
            return Ok(json!({ "ok": false, "error": err.trim() }));
        }
        return Ok(platform::win::parse_logs(&out));
    }
    let (_code, out, err) = ssh::exec(&s.handle, workspace::LOGS_CMD, Some(s.cancel.subscribe())).await?;
    let text = if out.trim().is_empty() { err } else { out };
    Ok(json!({ "ok": true, "text": text }))
}

#[tauri::command]
fn session_ki_respond(state: State<'_, AppState>, id: String, answers: Vec<String>) {
    if let Some(tx) = crate::sync::lock(&state.ki).remove(&id) {
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
    remote_fs::list(&s.remote_fs, &s.handle, &path).await
}
#[tauri::command]
async fn sftp_mkdir(state: State<'_, AppState>, session_id: String, path: String) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::mkdir(&s.remote_fs, &s.handle, &path).await
}
#[tauri::command]
async fn sftp_remove(state: State<'_, AppState>, session_id: String, path: String, is_dir: bool) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::remove(&s.remote_fs, &s.handle, &path, is_dir).await
}
#[tauri::command]
async fn sftp_rename(state: State<'_, AppState>, session_id: String, from: String, to: String) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::rename(&s.remote_fs, &s.handle, &from, &to).await
}
#[tauri::command]
async fn sftp_chmod(state: State<'_, AppState>, session_id: String, path: String, mode: u32) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::chmod(&s.remote_fs, &s.handle, &path, mode).await
}
#[tauri::command]
async fn sftp_preview(state: State<'_, AppState>, session_id: String, remote_path: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::preview(&s.remote_fs, &s.handle, &remote_path).await
}
#[tauri::command]
async fn sftp_read_file(state: State<'_, AppState>, session_id: String, remote_path: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::read_file(&s.remote_fs, &s.handle, &remote_path).await
}
#[tauri::command]
async fn sftp_write_file(state: State<'_, AppState>, session_id: String, remote_path: String, content: String, mode: u32, base_mtime: u64, eol: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::write_file(&s.remote_fs, &s.handle, &remote_path, &content, mode, base_mtime, &eol).await
}
#[tauri::command]
async fn sftp_name_conflicts(
    state: State<'_, AppState>,
    session_id: String,
    remote_dir: String,
    names: Vec<String>,
) -> Result<Vec<String>, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::name_conflicts(&s.remote_fs, &s.handle, &remote_dir, &names).await
}
#[tauri::command]
async fn sftp_upload_paths(app: AppHandle, state: State<'_, AppState>, session_id: String, remote_dir: String, paths: Vec<String>) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let n = paths.len();
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
    Ok(json!({ "uploaded": n }))
}
#[tauri::command]
async fn sftp_download_to(app: AppHandle, state: State<'_, AppState>, session_id: String, remote_path: String, local_dir: String) -> Result<(), String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    remote_fs::download_path(app, s.remote_fs.clone(), s.handle.clone(), &session_id, &remote_path, &local_dir, s.alive.clone(), state.transfers.clone()).await
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
    state.edit.open(app, s.handle.clone(), s.remote_fs.clone(), session_id, remote_path).await
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
#[tauri::command]
fn servers_import_mobaxterm() -> Result<Value, String> {
    Ok(json!({ "imported": importers::import_mobaxterm()? }))
}
#[tauri::command]
fn servers_import_xshell() -> Result<Value, String> {
    Ok(json!({ "imported": importers::import_xshell()? }))
}
#[tauri::command]
fn servers_import_securecrt() -> Result<Value, String> {
    Ok(json!({ "imported": importers::import_securecrt()? }))
}

// ---------------- Утилиты (P2.1) ----------------

#[tauri::command]
async fn tools_port_test(host: String, port: u16, timeout_ms: Option<u64>) -> Result<Value, String> {
    tools::port_test(host, port, timeout_ms).await
}
#[tauri::command]
async fn tools_dns_lookup(name: String) -> Result<Value, String> {
    tools::dns_lookup(name).await
}
#[tauri::command]
async fn tools_tls_cert(host: String, port: Option<u16>) -> Result<Value, String> {
    tools::tls_cert(host, port).await
}
/// HTTP-запрос со своей машины: код ответа, заголовки, время и цепочка переходов.
#[tauri::command]
async fn tools_http(
    url: String,
    method: Option<String>,
    max_redirects: Option<u8>,
) -> Result<Value, String> {
    tools::http_probe(url, method, max_redirects).await
}

/// HTTP-запрос **с сервера**: отвечает ли служба именно ему.
#[tauri::command]
async fn tools_http_on(
    state: State<'_, AppState>,
    session_id: String,
    url: String,
    method: Option<String>,
) -> Result<Value, String> {
    // Адрес уходит в командную строку, поэтому проверяем его тем же разбором, что и для
    // своей стороны: узел через `check_host`, схема - только http и https.
    let u = tools::parse_url(&url)?;
    let method = method.unwrap_or_else(|| "GET".into()).to_uppercase();
    if !matches!(method.as_str(), "GET" | "HEAD") {
        return Err("Пока умеем только GET и HEAD".into());
    }
    let целый = format!(
        "{}://{}:{}{}",
        if u.secure { "https" } else { "http" },
        u.host,
        u.port,
        u.path
    );
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    if kind == platform::Kind::Windows {
        return Err("HTTP-запрос с Windows-сервера пока не поддержан".into());
    }
    let cmd = tools::remote::http_cmd_posix(&целый, &method, 10);
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(tools::remote::parse_http(&целый, &out))
}

/// Откуда брать файл для сравнения.
///
/// Смысл утилиты именно в разнородности сторон: сравнить конфиг на двух серверах или
/// локальную правку с тем, что доехало, - вопросы, которые задают чаще всего, и ни один
/// из них не решается сравнением двух файлов на одной машине.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiffSide {
    /// Пусто - файл на этой машине; иначе идентификатор открытой SSH-сессии.
    #[serde(default)]
    session_id: Option<String>,
    path: String,
}

impl DiffSide {
    /// Подпись стороны для показа: путь и, если это сервер, откуда он.
    fn label(&self) -> String {
        match self.session_id.as_deref().filter(|s| !s.is_empty()) {
            Some(_) => format!("сервер: {}", self.path),
            None => format!("эта машина: {}", self.path),
        }
    }
}

async fn diff_side_text(state: &State<'_, AppState>, side: &DiffSide) -> Result<String, String> {
    match side.session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => {
            let s = state.ssh(id).ok_or("Сессия не подключена")?;
            let v = remote_fs::read_file(&s.remote_fs, &s.handle, &side.path).await?;
            // Слишком большой файл читалка отдаёт с пометкой и без содержимого. Сравнивать
            // обрезанное значило бы показать различия, которых в файлах нет.
            if v.get("tooLarge").and_then(|b| b.as_bool()).unwrap_or(false) {
                return Err(format!("Файл {} слишком большой для сравнения", side.path));
            }
            Ok(v.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string())
        }
        None => tokio::fs::read_to_string(&side.path)
            .await
            .map_err(|e| format!("Не удалось прочитать {}: {e}", side.path)),
    }
}

/// Сравнение двух файлов. Каждая сторона - эта машина или любая открытая сессия.
#[tauri::command]
async fn tools_diff(
    state: State<'_, AppState>,
    a: DiffSide,
    b: DiffSide,
) -> Result<Value, String> {
    let (ta, tb) = (diff_side_text(&state, &a).await?, diff_side_text(&state, &b).await?);
    // Двоичные файлы не сравниваем построчно: получился бы мусор, не отвечающий ни на
    // один вопрос. Но сказать, совпадают ли они, всё равно можем.
    if filediff::looks_binary(&ta) || filediff::looks_binary(&tb) {
        return Ok(json!({
            "a": a.label(),
            "b": b.label(),
            "same": ta == tb,
            "binary": true,
            "note": "Похоже на двоичные файлы - построчное сравнение для них бессмысленно",
        }));
    }
    Ok(filediff::compare(&a.label(), &ta, &b.label(), &tb))
}

/// Запрос к каталогу LDAP.
///
/// Только со своей машины: варианта «с сервера» здесь нет, и это осознанно. LDAP - это
/// ASN.1, готовый клиент открытый поток не принимает, а писать разбор протокола ради
/// второго варианта несоразмерно пользе. В интерфейсе об этом сказано прямо.
#[tauri::command]
async fn tools_ldap(params: ldap::Params) -> Result<Value, String> {
    ldap::search(params).await
}

/// Маршрут до адреса со своей машины.
#[tauri::command]
async fn tools_trace(host: String, hops: Option<u8>) -> Result<Value, String> {
    tools::trace(host, hops).await
}

/// Маршрут до адреса **с сервера**: у него свои маршруты, и это как раз тот случай,
/// когда ответ со своей машины ничего не говорит о чужой.
#[tauri::command]
async fn tools_trace_on(
    state: State<'_, AppState>,
    session_id: String,
    host: String,
    hops: Option<u8>,
) -> Result<Value, String> {
    let (host, _) = tools::parse_host_port(&host, 0)?;
    tools::remote::check_host(&host)?;
    let hops = hops.unwrap_or(15).clamp(1, 30);
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = match kind {
        platform::Kind::Windows => tools::remote::trace_cmd_windows(&host, hops),
        _ => tools::remote::trace_cmd_posix(&host, hops),
    };
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(tools::remote::parse_trace(&host, &out))
}

/// Просмотр диапазона портов со своей машины.
#[tauri::command]
async fn tools_port_scan(
    host: String,
    from: u16,
    to: u16,
    timeout_ms: Option<u64>,
) -> Result<Value, String> {
    tools::port_scan(host, from, to, timeout_ms).await
}

/// Просмотр диапазона портов **с сервера**.
///
/// Проверки там идут по очереди, поэтому диапазон стоит держать узким: сотня портов на
/// недоступном хосте с секундным таймаутом - это полторы минуты ожидания.
#[tauri::command]
async fn tools_port_scan_on(
    state: State<'_, AppState>,
    session_id: String,
    host: String,
    from: u16,
    to: u16,
) -> Result<Value, String> {
    let (host, _) = tools::parse_host_port(&host, from)?;
    tools::remote::check_host(&host)?;
    let (from, to) = tools::parse_range(from, to)?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    if kind == platform::Kind::Windows {
        return Err("Просмотр диапазона портов с Windows-сервера пока не поддержан".into());
    }
    let cmd = tools::remote::scan_cmd_posix(&host, from, to, 1);
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(tools::remote::parse_scan(&host, from, to, &out))
}

/// Проверка порта **с сервера**, а не со своей машины.
///
/// Разница не косметическая: при разборе неполадки почти всегда важно, видит ли адрес
/// сам сервер, а не тот, кто на него смотрит. Набор утилит на серверах разный, поэтому
/// команда собирается под систему, а её ответ разбирается отдельно и под тестами.
#[tauri::command]
async fn tools_port_test_on(
    state: State<'_, AppState>,
    session_id: String,
    host: String,
    port: u16,
) -> Result<Value, String> {
    let (host, port) = tools::parse_host_port(&host, port)?;
    tools::remote::check_host(&host)?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = match kind {
        platform::Kind::Windows => tools::remote::port_cmd_windows(&host, port),
        _ => tools::remote::port_cmd_posix(&host, port, 3),
    };
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(tools::remote::parse_port(&host, port, &out))
}

/// Разрешение имени **с сервера**: у него свои DNS и свой `/etc/hosts`.
#[tauri::command]
async fn tools_dns_lookup_on(
    state: State<'_, AppState>,
    session_id: String,
    name: String,
) -> Result<Value, String> {
    let name = name.trim().trim_end_matches('.').to_string();
    tools::remote::check_host(&name)?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = match kind {
        platform::Kind::Windows => tools::remote::dns_cmd_windows(&name),
        _ => tools::remote::dns_cmd_posix(&name),
    };
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(tools::remote::parse_dns(&name, &out))
}

#[tauri::command]
fn tools_subnet(input: String) -> Result<Value, String> {
    tools::subnet_calc(&input)
}
#[tauri::command]
fn tools_hash(algo: String, text: String) -> Result<Value, String> {
    tools::hash_text(&algo, &text)
}
#[tauri::command]
fn tools_jwt_decode(token: String) -> Result<Value, String> {
    tools::jwt_decode(&token)
}

/// Сдвинуть окна группы на (dx, dy) в физических пикселях.
/// Делаем из Rust: на Linux JS `setPosition` из чужого webview часто не доезжает,
/// а emit `serein-dock-move` сам по себе окна не двигает - только помечает «это наше».
#[tauri::command]
fn windows_nudge_group(app: AppHandle, members: Vec<String>, dx: i32, dy: i32) {
    if dx == 0 && dy == 0 {
        return;
    }
    use tauri::Manager;
    for label in members {
        let Some(w) = app.webview_windows().get(&label).cloned() else {
            continue;
        };
        let Ok(pos) = w.outer_position() else {
            continue;
        };
        let _ = w.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
            x: pos.x.saturating_add(dx),
            y: pos.y.saturating_add(dy),
        }));
    }
}

/// Поднять все окна приложения над чужими, фокус оставить на `focused`.
#[tauri::command]
fn windows_raise_group(app: AppHandle, focused: String) {
    windows_raise_group_impl(&app, &focused);
}

/// Развернуть все свёрнутые окна приложения (для режима «одна кнопка на панели задач»).
#[tauri::command]
fn windows_restore_minimized(app: AppHandle) -> u32 {
    windows_restore_minimized_impl(&app)
}

/// Сколько окон приложения сейчас свёрнуто.
#[tauri::command]
fn windows_count_minimized(app: AppHandle) -> u32 {
    windows_count_minimized_impl(&app)
}

// На Linux те же три операции делаются средствами самого Tauri. Раньше они там были
// заглушками, из-за чего «одна кнопка на панели задач» и подъём группы не работали
// вовсе: свёрнутые окна не разворачивались, а клик по одному окну не поднимал остальные.

#[cfg(not(windows))]
fn windows_count_minimized_impl(app: &AppHandle) -> u32 {
    app.webview_windows()
        .values()
        .filter(|w| w.is_minimized().unwrap_or(false))
        .count() as u32
}

#[cfg(not(windows))]
fn windows_restore_minimized_impl(app: &AppHandle) -> u32 {
    let mut n = 0;
    for w in app.webview_windows().values() {
        if w.is_minimized().unwrap_or(false) {
            let _ = w.unminimize();
            n += 1;
        }
    }
    n
}

#[cfg(not(windows))]
fn windows_raise_group_impl(app: &AppHandle, focused: &str) {
    // В Win32 есть отдельное «поднять, не забирая фокус» (SWP_NOACTIVATE); в Tauri его нет,
    // а `set_focus` перетащил бы фокус на каждое окно по очереди и заставил их мигать.
    // Переносимый эквивалент - короткое «поверх всех» и обратно: менеджер окон поднимает
    // окно, фокус остаётся там, где был.
    let windows = app.webview_windows();
    for (label, w) in &windows {
        if label == focused || w.is_minimized().unwrap_or(false) {
            continue;
        }
        let _ = w.set_always_on_top(true);
        let _ = w.set_always_on_top(false);
    }
    // Фокусируемое поднимаем последним, чтобы оно осталось верхним и активным.
    if let Some(w) = windows.get(focused) {
        let _ = w.set_focus();
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
        // Свернутые окна не трогаем - иначе minimize сразу отменяется raise_group.
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

/// WebView2 по умолчанию вешает Ctrl+Shift+C на Inspect - это ломает копирование в терминале.
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
            // Схему профиля приводим к текущей до того, как что-либо его прочитает.
            // Ошибка здесь означает профиль от более новой версии: продолжать нельзя -
            // первая же запись выбросит поля, которых мы не знаем.
            if let Err(e) = schema::migrate(&store::config_dir()) {
                use tauri_plugin_dialog::DialogExt;
                let _ = app.dialog()
                    .message(&e)
                    .title("Serein - профиль несовместим")
                    .blocking_show();
                return Err(e.into());
            }
            app.manage(AppState::new());
            #[cfg(windows)]
            if let Some(w) = app.get_webview_window("main") {
                disable_browser_accelerators(&w);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings_get, settings_set,
            servers_list, servers_save, servers_delete, servers_reorder,
            snippets_list, snippets_save, snippets_delete,
            layout_get, layout_set, aux_layout_get, aux_layout_set,
            localfs_home, localfs_parent, localfs_list, localfs_copy_into,
            session_open_local, session_open_ssh, session_write, session_resize, session_close,
            session_ping, session_replay, session_claim, session_monitor, session_ki_respond,
            session_log_status, session_log_toggle, ssh_agent_identities,
            session_hostkey_respond, knownhosts_list, knownhosts_forget, knownhosts_import,
            serial_ports, session_open_serial, serial_send_break, serial_set_signal,
            session_open_tcp, telnet_command,
            docker_list, docker_action, docker_logs, docker_stats, docker_logs_cancel, docker_container_files,
            docker_compose_list, docker_compose_ps, docker_compose_action, docker_compose_read,
            docker_compose_logs, docker_compose_logs_cancel,
            sftp_list, sftp_mkdir, sftp_remove, sftp_rename, sftp_chmod, sftp_preview, sftp_read_file, sftp_write_file,
            sftp_upload_paths, sftp_download_to, sftp_drag_out, sftp_name_conflicts, sftp_edit, sftp_edit_stop, sftp_cancel_transfer,
            sftp_pause_transfer, sftp_resume_transfer,
            tunnel_list_status, tunnel_open, tunnel_close,
            workspace_processes, workspace_kill, workspace_services, workspace_service_action, workspace_logs,
            workspace_platform,
            vnc_open, vnc_pointer, vnc_key, vnc_refresh, vnc_paste, vnc_close,
            rdp_open, rdp_pointer, rdp_key, rdp_resize, rdp_close, rdp_note, rdp_attach,
            desktop_active, vnc_attach,
            desktop_rdp_detect, desktop_rdp_install, desktop_rdp_start,
            desktop_detect, desktop_install, desktop_set_password,
            db_open, db_query, db_close, db_current,
            session_sysinfo,
            vault_status, vault_unlock, vault_enable, vault_disable,
            backup_export, backup_import,
            export_text_file,
            keygen_generate, keygen_save, keygen_install,
            servers_import_ssh_config, servers_import_putty,
            servers_import_mobaxterm, servers_import_xshell, servers_import_securecrt,
            tools_port_test, tools_dns_lookup, tools_tls_cert, tools_subnet, tools_hash, tools_jwt_decode,
            tools_port_test_on, tools_dns_lookup_on, tools_port_scan, tools_port_scan_on, tools_trace, tools_trace_on, tools_http, tools_http_on, tools_ldap, tools_diff,
            app_platform, app_paths, app_install_kind, multi_exec, multi_exec_cancel,
            windows_nudge_group, windows_raise_group, windows_restore_minimized, windows_count_minimized,
            clipboard_write, clipboard_read
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
