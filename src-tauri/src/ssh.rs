//! SSH-сессии через russh: интерактивный shell + exec + ProxyJump + keyboard-interactive 2FA.
//! Handle сохраняется живым, чтобы из него открывать exec-каналы (мониторинг/docker/ping),
//! SFTP и туннели.

use crate::knownhosts;
use crate::ssh_agent;
use russh::client::{self, Handler, KeyboardInteractiveAuthResponse, Msg, Session};
use russh::{Channel, ChannelId, ChannelMsg, CryptoVec};
use russh_keys::{load_secret_key, PublicKeyBase64};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;
use tokio::sync::watch;

/// Маршрутизация remote-форвардов (-R): remote_port → local_port на этом соединении.
pub type RemoteForwards = Arc<Mutex<HashMap<u32, u16>>>;

/// Мост keyboard-interactive: sessionId → канал доставки ответов из renderer.
pub type KiBridge = Arc<Mutex<HashMap<String, oneshot::Sender<Vec<String>>>>>;

/// true = сессию гасим: туннельные copy и SFTP выходят из select/цикла.
pub type CancelRx = watch::Receiver<bool>;

pub async fn wait_cancel(mut rx: CancelRx) {
    loop {
        if *rx.borrow() {
            return;
        }
        if rx.changed().await.is_err() {
            return;
        }
    }
}

/// true, как только сработал любой из двух флагов.
pub fn race_cancel(a: CancelRx, b: CancelRx) -> CancelRx {
    let (tx, rx) = watch::channel(false);
    tauri::async_runtime::spawn(async move {
        tokio::select! {
            _ = wait_cancel(a) => {}
            _ = wait_cancel(b) => {}
        }
        let _ = tx.send(true);
    });
    rx
}

pub enum SshCmd {
    Write(Vec<u8>),
    Resize(u32, u32),
    Close,
}

/// Целевой Handle за Mutex: &self-операции (открытие каналов) лочат его кратко,
/// а tcpip_forward/cancel (-R) требуют &mut — лочат на время вызова.
pub type SharedHandle = Arc<tokio::sync::Mutex<client::Handle<ClientHandler>>>;

pub struct SshSession {
    pub handle: SharedHandle,
    pub tx: UnboundedSender<SshCmd>,
    pub server_id: String,
    pub remote_forwards: RemoteForwards,
    /// Промежуточные клиенты цепочки jump-хостов — держим живыми до закрытия сессии.
    pub jump_handles: Vec<Arc<client::Handle<ClientHandler>>>,
    /// true, если фронт сам вызвал session_close — не считать обрывом.
    pub user_closed: Arc<AtomicBool>,
    /// false после shutdown — SFTP/exec не крутятся до таймаута.
    pub alive: Arc<AtomicBool>,
    pub cancel: watch::Sender<bool>,
}

impl SshSession {
    /// Пометить мёртвой, остановить туннели/SFTP, разорвать russh. Идемпотентно по смыслу флагов.
    pub fn shutdown(&self, user: bool) {
        if user {
            self.user_closed.store(true, Ordering::Relaxed);
        }
        self.alive.store(false, Ordering::Relaxed);
        let _ = self.cancel.send(true);
        self.remote_forwards.lock().unwrap().clear();
        let _ = self.tx.send(SshCmd::Close);
        let handle = self.handle.clone();
        let jumps = self.jump_handles.clone();
        tauri::async_runtime::spawn(async move {
            {
                let h = handle.lock().await;
                let _ = h
                    .disconnect(russh::Disconnect::ByApplication, "session close", "")
                    .await;
            }
            for j in jumps {
                let _ = j
                    .disconnect(russh::Disconnect::ByApplication, "session close", "")
                    .await;
            }
        });
    }
}

pub struct ClientHandler {
    host_id: String,
    remote_forwards: RemoteForwards,
    cancel: CancelRx,
    /// Каналы agent forwarding → локальный ssh-agent.
    agent_forward_channels: Arc<Mutex<HashSet<ChannelId>>>,
    agent_lock: Arc<tokio::sync::Mutex<()>>,
}

#[async_trait::async_trait]
impl Handler for ClientHandler {
    type Error = russh::Error;

    /// TOFU-проверка ключа сервера (known_hosts).
    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fp = knownhosts::fingerprint_from_b64(&server_public_key.public_key_base64());
        Ok(knownhosts::check_and_remember(&self.host_id, &fp))
    }

    /// Входящее соединение по remote-форварду (-R): маршрутизируем на локальный порт.
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let local_port = self
            .remote_forwards
            .lock()
            .unwrap()
            .get(&connected_port)
            .copied();
        if let Some(lp) = local_port {
            let cancel = self.cancel.clone();
            tokio::spawn(async move {
                if let Ok(mut tcp) = tokio::net::TcpStream::connect(("127.0.0.1", lp)).await {
                    let mut stream = channel.into_stream();
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream) => {}
                        _ = wait_cancel(cancel) => {}
                    }
                }
            });
        }
        Ok(())
    }

    /// Проброс SSH-агента: сервер открыл канал auth-agent@openssh.com.
    async fn server_channel_open_agent_forward(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.agent_forward_channels.lock().unwrap().insert(channel);
        Ok(())
    }

    /// Данные с канала agent forwarding → локальный ssh-agent → ответ обратно.
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if !self.agent_forward_channels.lock().unwrap().contains(&channel) {
            return Ok(());
        }
        let _guard = self.agent_lock.lock().await;
        match ssh_agent::agent_roundtrip(data).await {
            Ok(reply) => session.data(channel, CryptoVec::from(reply)),
            Err(_) => session.close(channel),
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.agent_forward_channels.lock().unwrap().remove(&channel);
        Ok(())
    }
}

fn field<'a>(server: &'a Value, key: &str) -> Option<&'a str> {
    server.get(key).and_then(|v| v.as_str())
}

fn port_of(server: &Value) -> u16 {
    server.get("port").and_then(|v| v.as_u64()).unwrap_or(22) as u16
}

/// Окно и keepalive под длинные SFTP. `maximum_packet_size` 32 КиБ — как у OpenSSH;
/// SFTP-чанк в `sftp.rs` режется под этот лимит, иначе DATA не влезает в SSH-пакет.
pub(crate) fn ssh_client_config() -> Arc<client::Config> {
    let mut cfg = client::Config::default();
    cfg.window_size = 32 * 1024 * 1024;
    cfg.maximum_packet_size = 32 * 1024;
    cfg.keepalive_interval = Some(std::time::Duration::from_secs(15));
    cfg.keepalive_max = 8;
    Arc::new(cfg)
}

async fn request_ki(app: &AppHandle, ki: &KiBridge, id: &str, prompts: Vec<Value>) -> Vec<String> {
    let (tx, rx) = oneshot::channel();
    ki.lock().unwrap().insert(id.to_string(), tx);
    let _ = app.emit("session-ki", json!({ "id": id, "prompts": prompts }));
    rx.await.unwrap_or_default()
}

/// Аутентификация одного хопа. Для целевого сервера (есть `id`/`ki`) — с поддержкой 2FA.
fn wants_agent_forward(server: &Value) -> bool {
    server
        .get("agentForward")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || field(server, "authType") == Some("agent")
}

async fn authenticate(
    handle: &mut client::Handle<ClientHandler>,
    server: &Value,
    app: Option<&AppHandle>,
    ki: &KiBridge,
    id: Option<&str>,
) -> Result<bool, String> {
    let user = field(server, "username").unwrap_or("root").to_string();
    let auth_type = field(server, "authType").unwrap_or("password");

    match auth_type {
        "key" => {
            let path = field(server, "privateKeyPath").ok_or("Не задан путь к ключу")?;
            let passphrase = field(server, "passphrase");
            let key = load_secret_key(path, passphrase).map_err(|e| e.to_string())?;
            handle
                .authenticate_publickey(&user, Arc::new(key))
                .await
                .map_err(|e| e.to_string())
        }
        "agent" => ssh_agent::authenticate_with_agent(handle, &user).await,
        _ => {
            // password (+ фолбэк на keyboard-interactive/2FA для целевого сервера).
            let pass = field(server, "password").unwrap_or("");
            if !pass.is_empty() {
                if handle
                    .authenticate_password(&user, pass)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    return Ok(true);
                }
            }
            // keyboard-interactive (2FA/OTP) — только для целевого сервера.
            if let Some(sid) = id {
                let mut resp = handle
                    .authenticate_keyboard_interactive_start(&user, None)
                    .await
                    .map_err(|e| e.to_string())?;
                loop {
                    match resp {
                        KeyboardInteractiveAuthResponse::Success => return Ok(true),
                        KeyboardInteractiveAuthResponse::Failure => return Ok(false),
                        KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                            let pl: Vec<Value> = prompts
                                .iter()
                                .map(|p| json!({ "prompt": p.prompt, "echo": p.echo }))
                                .collect();
                            let Some(app) = app else {
                                return Err("keyboard-interactive недоступен без UI".into());
                            };
                            let answers = request_ki(app, ki, sid, pl).await;
                            resp = handle
                                .authenticate_keyboard_interactive_respond(answers)
                                .await
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
            Ok(false)
        }
    }
}

async fn connect_one(
    server: &Value,
    rf: RemoteForwards,
    cancel: CancelRx,
) -> Result<client::Handle<ClientHandler>, String> {
    let host = field(server, "host").ok_or("Не задан host")?;
    let handler = ClientHandler {
        host_id: knownhosts::host_id(host, port_of(server)),
        remote_forwards: rf,
        cancel,
        agent_forward_channels: Arc::new(Mutex::new(HashSet::new())),
        agent_lock: Arc::new(tokio::sync::Mutex::new(())),
    };
    let config = ssh_client_config();
    // Таймаут подключения: к офлайн/firewall-хосту (DROP без TCP-reset) connect иначе
    // виснет надолго. russh отдаёт ошибку Result (не EventEmitter, как ssh2 в Electron),
    // поэтому "двойного error" тут нет — достаточно ограничить ожидание и вернуть Err.
    let secs = server
        .get("connectTimeout")
        .and_then(|v| v.as_u64())
        .filter(|n| *n > 0)
        .unwrap_or(15);
    let fut = client::connect(config, (host, port_of(server)), handler);
    match tokio::time::timeout(std::time::Duration::from_secs(secs), fut).await {
        Ok(r) => r.map_err(|e| format!("Подключение к {host} не удалось: {e}")),
        Err(_) => Err(format!("Подключение к {host}: таймаут {secs}с (хост недоступен?)")),
    }
}

/// Подключается по цепочке: chain[0] — цель, chain[1..] — jump-хосты (как в Electron).
pub async fn connect_chain(
    app: AppHandle,
    id: String,
    chain: Vec<Value>,
    cols: u32,
    rows: u32,
    ki: KiBridge,
) -> Result<SshSession, String> {
    if chain.is_empty() {
        return Err("Пустая цепочка хостов".into());
    }
    let target = chain[0].clone();
    let mut jump_handles: Vec<Arc<client::Handle<ClientHandler>>> = Vec::new();
    let remote_forwards: RemoteForwards = Arc::new(Mutex::new(HashMap::new()));
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let alive = Arc::new(AtomicBool::new(true));

    // Самый дальний хоп (конец цепочки) — прямое подключение.
    let far = chain.last().unwrap();
    let mut handle = connect_one(far, remote_forwards.clone(), cancel_rx.clone()).await?;
    let far_is_target = chain.len() == 1;
    if !authenticate(
        &mut handle,
        far,
        Some(&app),
        &ki,
        if far_is_target { Some(id.as_str()) } else { None },
    )
    .await?
    {
        return Err("Аутентификация отклонена сервером".into());
    }

    // Идём внутрь к цели: на каждом шаге пробрасываем direct-tcpip и подключаемся поверх.
    let mut cur = handle;
    for i in (0..chain.len() - 1).rev() {
        let next = &chain[i];
        let nhost = field(next, "host").ok_or("Не задан host промежуточного хоста")?;
        let channel = cur
            .channel_open_direct_tcpip(nhost, port_of(next) as u32, "127.0.0.1", 0)
            .await
            .map_err(|e| format!("ProxyJump к {nhost} не удался: {e}"))?;
        jump_handles.push(Arc::new(cur));
        let config = ssh_client_config();
        let handler = ClientHandler {
            host_id: knownhosts::host_id(nhost, port_of(next)),
            remote_forwards: remote_forwards.clone(),
            cancel: cancel_rx.clone(),
            agent_forward_channels: Arc::new(Mutex::new(HashSet::new())),
            agent_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let mut nh = client::connect_stream(config, channel.into_stream(), handler)
            .await
            .map_err(|e| e.to_string())?;
        let is_target = i == 0;
        if !authenticate(
            &mut nh,
            next,
            Some(&app),
            &ki,
            if is_target { Some(id.as_str()) } else { None },
        )
        .await?
        {
            return Err("Аутентификация отклонена сервером".into());
        }
        cur = nh;
    }

    // cur — целевой клиент. Открываем shell.
    let mut channel = cur
        .channel_open_session()
        .await
        .map_err(|e| e.to_string())?;
    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .map_err(|e| e.to_string())?;
    if wants_agent_forward(&target) {
        channel.agent_forward(true).await.map_err(|e| e.to_string())?;
    }
    channel.request_shell(true).await.map_err(|e| e.to_string())?;

    let handle: SharedHandle = Arc::new(tokio::sync::Mutex::new(cur));
    let (tx, mut rx) = mpsc::unbounded_channel::<SshCmd>();
    let app2 = app.clone();
    let id2 = id.clone();
    let user_closed = Arc::new(AtomicBool::new(false));
    let user_closed2 = user_closed.clone();
    let out = crate::term_out::TermOut::spawn(app2.clone(), id2.clone());
    tokio::spawn(async move {
        loop {
            tokio::select! {
                msg = channel.wait() => {
                    match msg {
                        Some(ChannelMsg::Data { ref data }) => {
                            out.push(&data[..]);
                        }
                        Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                            out.push(&data[..]);
                        }
                        Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                        _ => {}
                    }
                }
                cmd = rx.recv() => {
                    match cmd {
                        Some(SshCmd::Write(d)) => { let _ = channel.data(&d[..]).await; }
                        Some(SshCmd::Resize(c, r)) => { let _ = channel.window_change(c, r, 0, 0).await; }
                        Some(SshCmd::Close) | None => break,
                    }
                }
            }
        }
        out.close();
        out.join().await;
        let _ = channel.close().await;
        let is_drop = !user_closed2.load(Ordering::Relaxed);
        let _ = app2.emit(
            "session-exit",
            json!({
                "id": id2,
                "code": 0,
                "signal": null,
                "reason": if is_drop { "drop" } else { "user" },
                "error": is_drop.then_some("Соединение разорвано"),
            }),
        );
        if let Some(st) = app2.try_state::<crate::AppState>() {
            st.teardown(&app2, &id2, !is_drop);
        }
    });

    Ok(SshSession {
        handle,
        tx,
        server_id: field(&target, "id").unwrap_or("").to_string(),
        remote_forwards,
        jump_handles,
        user_closed,
        alive,
        cancel: cancel_tx,
    })
}

/// Подключение без PTY/shell — для exec/SFTP (bench и служебные каналы).
pub async fn connect_client(chain: Vec<Value>) -> Result<SharedHandle, String> {
    if chain.is_empty() {
        return Err("Пустая цепочка хостов".into());
    }
    let dummy_ki: KiBridge = Arc::new(Mutex::new(HashMap::new()));
    let remote_forwards: RemoteForwards = Arc::new(Mutex::new(HashMap::new()));
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let far = chain.last().unwrap();
    let mut handle = connect_one(far, remote_forwards.clone(), cancel_rx.clone()).await?;
    if !authenticate(&mut handle, far, None, &dummy_ki, None).await? {
        return Err("Аутентификация отклонена сервером".into());
    }
    let mut cur = handle;
    for i in (0..chain.len() - 1).rev() {
        let next = &chain[i];
        let nhost = field(next, "host").ok_or("Не задан host промежуточного хоста")?;
        let channel = cur
            .channel_open_direct_tcpip(nhost, port_of(next) as u32, "127.0.0.1", 0)
            .await
            .map_err(|e| format!("ProxyJump к {nhost} не удался: {e}"))?;
        let config = ssh_client_config();
        let handler = ClientHandler {
            host_id: knownhosts::host_id(nhost, port_of(next)),
            remote_forwards: remote_forwards.clone(),
            cancel: cancel_rx.clone(),
            agent_forward_channels: Arc::new(Mutex::new(HashSet::new())),
            agent_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let mut nh = client::connect_stream(config, channel.into_stream(), handler)
            .await
            .map_err(|e| e.to_string())?;
        if !authenticate(&mut nh, next, None, &dummy_ki, None).await? {
            return Err("Аутентификация отклонена сервером".into());
        }
        cur = nh;
    }
    Ok(Arc::new(tokio::sync::Mutex::new(cur)))
}

/// Exec, который режет поток по времени (логи -f, yes). Возвращает байты stdout+stderr.
pub async fn exec_for(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    command: &str,
    limit: std::time::Duration,
) -> Result<(u64, i32), String> {
    let mut channel = {
        let h = handle.lock().await;
        h.channel_open_session().await.map_err(|e| e.to_string())?
    };
    channel.exec(true, command).await.map_err(|e| e.to_string())?;
    let mut bytes = 0u64;
    let mut code = 0i32;
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            let _ = channel.close().await;
            break;
        }
        match tokio::time::timeout(left, channel.wait()).await {
            Ok(Some(ChannelMsg::Data { ref data })) => bytes += data.len() as u64,
            Ok(Some(ChannelMsg::ExtendedData { ref data, .. })) => bytes += data.len() as u64,
            Ok(Some(ChannelMsg::ExitStatus { exit_status })) => code = exit_status as i32,
            Ok(Some(ChannelMsg::Eof)) => {}
            Ok(Some(ChannelMsg::Close) | None) => break,
            Ok(Some(_)) => {}
            Err(_) => {
                let _ = channel.close().await;
                break;
            }
        }
    }
    Ok((bytes, code))
}

/// Выполняет команду отдельным exec-каналом. Возвращает (код, stdout, stderr).
/// `cancel` — оборвать канал (теardown сессии / стоп логов).
pub async fn exec(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    command: &str,
    cancel: Option<CancelRx>,
) -> Result<(i32, String, String), String> {
    exec_with(handle, command, cancel, |_| {}).await
}

pub async fn exec_with(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    command: &str,
    cancel: Option<CancelRx>,
    mut on_out: impl FnMut(&[u8]),
) -> Result<(i32, String, String), String> {
    let mut channel = {
        let h = handle.lock().await;
        h.channel_open_session().await.map_err(|e| e.to_string())?
    };
    channel.exec(true, command).await.map_err(|e| e.to_string())?;
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let mut code = 0i32;
    let mut cancel = cancel;
    loop {
        let msg = if let Some(rx) = cancel.as_mut() {
            tokio::select! {
                m = channel.wait() => m,
                _ = wait_cancel(rx.clone()) => {
                    let _ = channel.close().await;
                    return Ok((130, String::from_utf8_lossy(&out).to_string(), String::from_utf8_lossy(&err).to_string()));
                }
            }
        } else {
            channel.wait().await
        };
        match msg {
            Some(ChannelMsg::Data { ref data }) => {
                on_out(&data[..]);
                if out.len() < 512 * 1024 {
                    out.extend_from_slice(&data[..]);
                }
            }
            Some(ChannelMsg::ExtendedData { ref data, ext }) => {
                if ext == 1 && err.len() < 128 * 1024 {
                    err.extend_from_slice(&data[..]);
                }
            }
            Some(ChannelMsg::ExitStatus { exit_status }) => code = exit_status as i32,
            Some(ChannelMsg::Eof) => {}
            Some(ChannelMsg::Close) | None => break,
            _ => {}
        }
    }
    Ok((
        code,
        String::from_utf8_lossy(&out).to_string(),
        String::from_utf8_lossy(&err).to_string(),
    ))
}

/// Round-trip exec "true" в миллисекундах.
pub async fn ping(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>) -> Option<u32> {
    let t = std::time::Instant::now();
    let _ = exec(handle, "true", None).await;
    Some(t.elapsed().as_millis() as u32)
}

#[derive(Default)]
pub struct OpHub {
    tx: Mutex<HashMap<String, watch::Sender<bool>>>,
}

impl OpHub {
    pub fn begin(&self, key: &str) -> CancelRx {
        let (tx, rx) = watch::channel(false);
        self.tx.lock().unwrap().insert(key.to_string(), tx);
        rx
    }

    pub fn cancel(&self, key: &str) {
        if let Some(tx) = self.tx.lock().unwrap().get(key) {
            let _ = tx.send(true);
        }
    }

    pub fn finish(&self, key: &str) {
        self.tx.lock().unwrap().remove(key);
    }

    pub fn cancel_prefix(&self, prefix: &str) {
        let guard = self.tx.lock().unwrap();
        for (k, tx) in guard.iter() {
            if k.starts_with(prefix) {
                let _ = tx.send(true);
            }
        }
    }
}
