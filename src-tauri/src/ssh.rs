//! SSH-сессии через russh: интерактивный shell + exec + ProxyJump + keyboard-interactive 2FA.
//! Handle сохраняется живым, чтобы из него открывать exec-каналы (мониторинг/docker/ping),
//! SFTP и туннели.

use crate::knownhosts;
use crate::ssh_agent;
use russh::client::{self, ChannelOpenHandle, Handler, KeyboardInteractiveAuthResponse, Msg, Session};
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg};
use russh::{Channel, ChannelMsg};
use serde_json::{json, Value};
use std::collections::HashMap;
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

/// Мост подтверждения ключа хоста: requestId → канал с ответом пользователя.
pub type HostKeyBridge = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;

/// Чем спросить пользователя про ключ хоста во время рукопожатия.
#[derive(Clone)]
pub struct HostKeyAsk {
    pub app: AppHandle,
    pub bridge: HostKeyBridge,
    /// id сессии - чтобы фронт понял, к какой вкладке относится вопрос.
    pub session_id: String,
    /// Часы подключения этой сессии: пока человек решает, срок не идёт.
    pub pause: HumanPause,
}

impl HostKeyAsk {
    /// Шлёт событие в UI и ждёт ответ. Обрыв канала (окно закрыли) = отказ:
    /// на вопрос о доверии молчание не может значить «да».
    pub async fn confirm(&self, host: &str, fingerprint: &str, kind: &str, previous: &str) -> bool {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        crate::sync::lock(&self.bridge).insert(request_id.clone(), tx);
        let _ = self.app.emit(
            "session-hostkey",
            json!({
                "id": self.session_id,
                "requestId": request_id,
                "host": host,
                "fingerprint": fingerprint,
                "previous": previous,
                "kind": kind,
            }),
        );
        let _waiting = self.pause.begin();
        rx.await.unwrap_or(false)
    }
}

/// Как относиться к ключу сервера, который не совпал с подтверждённым.
///
/// Раньше это была `Option<HostKeyAsk>`, и `None` означало «незнакомый ключ принять
/// молча». Так подключались Fleet, восстановление туннелей и служебные каналы - то есть
/// целый класс подключений доверял первому встречному, хотя `SECURITY.md` обещает
/// обратное. Теперь варианты названы словами, и молчаливого доверия среди них нет.
#[derive(Clone)]
pub enum Trust {
    /// Спросить человека: незнакомый ключ и смена ключа выносятся в окно сессии.
    Ask(HostKeyAsk),
    /// Только уже подтверждённый ключ. Фоновые подключения, где спросить некого: Fleet,
    /// восстановление туннелей, второе соединение рабочего стола.
    KnownOnly,
    /// Принять незнакомый ключ и запомнить. Включается только переменной окружения для
    /// стенда - в обычной работе этот вариант не выбирается нигде.
    AcceptNewForTests,
}

impl Trust {
    /// Решение по лазейке - отдельно и без чтения окружения: так его можно проверить
    /// тестом, не подменяя переменные всему процессу.
    fn from_override(requested: bool) -> Self {
        if requested {
            Trust::AcceptNewForTests
        } else {
            Trust::KnownOnly
        }
    }

    /// Просит ли окружение принимать незнакомые ключи. Только в отладочной сборке.
    ///
    /// В релизной сборке этой ветки нет вовсе - вместе с именем переменной. Раньше она
    /// работала и в поставляемом приложении: кто мог выставить переменную в сеансе
    /// пользователя (ярлык, скрипт запуска, родительский процесс), тот молча снимал
    /// проверку ключей хостов у Fleet, восстановления туннелей и задач.
    #[cfg(debug_assertions)]
    fn override_requested() -> bool {
        std::env::var_os("SEREIN_TRUST_NEW_HOSTS").is_some()
    }

    #[cfg(not(debug_assertions))]
    fn override_requested() -> bool {
        false
    }

    /// Политика для фоновых подключений.
    ///
    /// Обычно - строго по подтверждённым. Исключение одно, оно объявляется явно и живёт
    /// только в отладочной сборке: переменная `SEREIN_TRUST_NEW_HOSTS` для стенда, где
    /// серверы поднимаются заново на каждый прогон и подтверждать их вручную некому.
    pub fn background() -> Self {
        let trust = Self::from_override(Self::override_requested());
        if matches!(trust, Trust::AcceptNewForTests) {
            // Даже на стенде это должно оставлять след: иначе «почему подключилось к
            // неизвестному серверу» выясняется только чтением кода.
            crate::actionlog::record(
                None,
                None,
                "ssh.trust.test-mode",
                json!({ "var": "SEREIN_TRUST_NEW_HOSTS" }),
                Ok(()),
            );
        }
        trust
    }
}

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

/// Сколько ждём открытия канала.
///
/// Срок нужен потому, что замок сессии общий: пока он занят, встают и терминал, и файлы,
/// и замер отклика. Сервер, который принял соединение и замолчал на открытии канала,
/// держал этот замок сколько угодно - вся рабочая область выглядела зависшей, и понять,
/// что именно висит, было нельзя. Пятнадцать секунд и внятный отказ лучше.
pub const CHANNEL_OPEN_LIMIT: std::time::Duration = std::time::Duration::from_secs(15);

/// Ждёт открытия канала не дольше отведённого срока.
async fn within_limit<T>(
    what: &str,
    fut: impl std::future::Future<Output = Result<T, russh::Error>>,
) -> Result<T, String> {
    match tokio::time::timeout(CHANNEL_OPEN_LIMIT, fut).await {
        Ok(r) => r.map_err(|e| format!("{what} не открылся: {e}")),
        Err(_) => Err(format!(
            "{what} не открылся за {} с - сервер не ответил",
            CHANNEL_OPEN_LIMIT.as_secs()
        )),
    }
}

/// Сколько ждём ответа сервера на вход одного хопа: пароль, ключ, агент, шаги второго
/// фактора. Время, пока человек вводит код, сюда не входит.
pub const AUTH_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Часы подключения, которые стоят, пока ждём человека.
///
/// Сроки на подключение нужны против сервера, который принял соединение и замолчал. Но
/// посреди того же рукопожатия приложение спрашивает человека - доверять ли ключу, какой
/// код второго фактора, - и прежний срок в 15 секунд обрывал подключение, пока человек
/// читал отпечаток. Время ответа человека в срок машины не засчитывается.
#[derive(Clone, Default)]
pub struct HumanPause(Arc<Mutex<PauseState>>);

#[derive(Default)]
struct PauseState {
    waiting: usize,
    since: Option<std::time::Instant>,
    total: std::time::Duration,
}

/// Пока жив - ждём человека.
pub struct PauseGuard(HumanPause);

impl HumanPause {
    pub fn begin(&self) -> PauseGuard {
        let mut st = crate::sync::lock(&self.0);
        if st.waiting == 0 {
            st.since = Some(std::time::Instant::now());
        }
        st.waiting += 1;
        drop(st);
        PauseGuard(self.clone())
    }

    /// Сколько всего ждали человека, включая ожидание прямо сейчас, и ждём ли сейчас.
    fn state(&self) -> (std::time::Duration, bool) {
        let st = crate::sync::lock(&self.0);
        let now = st.since.map(|t| t.elapsed()).unwrap_or_default();
        (st.total + now, st.waiting > 0)
    }
}

impl Drop for PauseGuard {
    fn drop(&mut self) {
        let mut st = crate::sync::lock(&(self.0).0);
        st.waiting = st.waiting.saturating_sub(1);
        if st.waiting == 0 {
            if let Some(t) = st.since.take() {
                st.total += t.elapsed();
            }
        }
    }
}

/// Ждёт будущее не дольше `limit` машинного времени. `Err(())` - срок вышел.
async fn machine_limit<T>(
    pause: &HumanPause,
    limit: std::time::Duration,
    fut: impl std::future::Future<Output = T>,
) -> Result<T, ()> {
    tokio::pin!(fut);
    let started = std::time::Instant::now();
    let (human_before, _) = pause.state();
    loop {
        let (human, waiting) = pause.state();
        let used = started.elapsed().saturating_sub(human.saturating_sub(human_before));
        let nap = if waiting {
            std::time::Duration::from_millis(250)
        } else if used >= limit {
            return Err(());
        } else {
            limit - used
        };
        tokio::select! {
            r = &mut fut => return Ok(r),
            _ = tokio::time::sleep(nap) => {}
        }
    }
}

/// Срок на подключение к серверу из его настроек, по умолчанию 15 с.
fn connect_limit(server: &Value) -> (u64, std::time::Duration) {
    let secs = server
        .get("connectTimeout")
        .and_then(|v| v.as_u64())
        .filter(|n| *n > 0)
        .unwrap_or(15);
    (secs, std::time::Duration::from_secs(secs))
}

/// Канал сессии (exec, shell, SFTP, SCP) под общим замком, но с ограничением по времени.
pub async fn open_session_channel(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
) -> Result<Channel<Msg>, String> {
    let g = handle.lock().await;
    within_limit("канал сессии", g.channel_open_session()).await
}

/// Канал `direct-tcpip` до `host:port` со стороны сервера - тем же способом.
pub async fn open_forward_channel(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    host: &str,
    port: u16,
) -> Result<Channel<Msg>, String> {
    let g = handle.lock().await;
    // «127.0.0.1» здесь - петля сервера, а не наша: канал открывает удалённая сторона.
    within_limit(
        &format!("канал до {host}:{port}"),
        g.channel_open_direct_tcpip(host, u32::from(port), "127.0.0.1", 0),
    )
    .await
}

/// Целевой Handle за Mutex: &self-операции (открытие каналов) лочат его кратко,
/// а tcpip_forward/cancel (-R) требуют &mut - лочат на время вызова.
pub type SharedHandle = Arc<tokio::sync::Mutex<client::Handle<ClientHandler>>>;

pub struct SshSession {
    pub handle: SharedHandle,
    pub tx: UnboundedSender<SshCmd>,
    pub server_id: String,
    pub remote_forwards: RemoteForwards,
    /// Промежуточные клиенты цепочки jump-хостов - держим живыми до закрытия сессии.
    pub jump_handles: Vec<Arc<client::Handle<ClientHandler>>>,
    /// true, если фронт сам вызвал session_close - не считать обрывом.
    pub user_closed: Arc<AtomicBool>,
    /// false после shutdown - SFTP/exec не крутятся до таймаута.
    pub alive: Arc<AtomicBool>,
    pub cancel: watch::Sender<bool>,
    /// SFTP или SCP - определяется при первой файловой операции.
    pub remote_fs: Arc<Mutex<crate::remote_fs::SessionFs>>,
}

impl SshSession {
    /// Пометить мёртвой, остановить туннели/SFTP, разорвать russh. Идемпотентно по смыслу флагов.
    pub fn shutdown(&self, user: bool) {
        if user {
            self.user_closed.store(true, Ordering::Relaxed);
        }
        self.alive.store(false, Ordering::Relaxed);
        let _ = self.cancel.send(true);
        crate::sync::lock(&self.remote_forwards).clear();
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
    /// Что делать с ключом, которого нет среди подтверждённых.
    trust: Trust,
    remote_forwards: RemoteForwards,
    cancel: CancelRx,
    agent_lock: Arc<tokio::sync::Mutex<()>>,
}

// В russh 0.63 `Handler` объявлен через `impl Future` в самом трейте, а не через
// `#[async_trait]`. Атрибут здесь теперь ломает сигнатуры: он переписывает время жизни
// ссылок, и они перестают совпадать с объявлением.
impl Handler for ClientHandler {
    type Error = russh::Error;

    /// Проверка ключа сервера. Незнакомый ключ и смена ключа выносятся пользователю:
    /// молча доверять первому встречному - это TOFU без буквы T, а молча рвать соединение
    /// при смене ключа выглядит как «непонятная ошибка сети».
    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // Сертификаты хостов мы не запрашиваем (`host_key_certificates` пуст), поэтому сюда
        // приходит обычный ключ. Если сервер всё же прислал сертификат - доверять ему без
        // списка удостоверяющих ключей нельзя, отказываем.
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            return Ok(false);
        };
        let fp = knownhosts::fingerprint_from_b64(&base64_of(key));
        let status = knownhosts::status(&self.host_id, &fp);

        if status == knownhosts::HostKeyStatus::Trusted {
            return Ok(true);
        }
        // Хранилище отпечатков не прочитать - отказываемся подключаться вообще. Считать в
        // этот момент сервер незнакомым нельзя: именно так подмена ключа и выглядела бы.
        if let knownhosts::HostKeyStatus::Unreadable { why } = &status {
            crate::rdp::log(&format!(
                "хранилище отпечатков не прочитать ({why}) - подключение к {} отклонено",
                self.host_id
            ));
            return Ok(false);
        }

        let ask = match &self.trust {
            Trust::Ask(ask) => ask.clone(),
            // Незнакомый ключ в фоновом подключении - причина отказаться, а не доверять.
            Trust::KnownOnly => return Ok(false),
            Trust::AcceptNewForTests => {
                if matches!(status, knownhosts::HostKeyStatus::New) {
                    let _ = knownhosts::remember(&self.host_id, &fp);
                    return Ok(true);
                }
                // Смена ключа не принимается даже на стенде: этот случай тесты и проверяют.
                return Ok(false);
            }
        };

        let (kind, previous) = match &status {
            knownhosts::HostKeyStatus::New => ("new", String::new()),
            knownhosts::HostKeyStatus::Changed { previous } => ("changed", previous.clone()),
            knownhosts::HostKeyStatus::Trusted | knownhosts::HostKeyStatus::Unreadable { .. } => {
                unreachable!("обработано выше")
            }
        };

        let accepted = ask.confirm(&self.host_id, &fp, kind, &previous).await;
        if accepted {
            // Не запомнили - об этом надо знать: иначе в следующий раз спросят снова, и
            // человек привыкает нажимать «доверяю», не вчитываясь.
            if let Err(why) = knownhosts::remember(&self.host_id, &fp) {
                crate::rdp::log(&format!("отпечаток {} не сохранён: {why}", self.host_id));
            }
        }
        Ok(accepted)
    }

    /// Входящее соединение по remote-форварду (-R): маршрутизируем на локальный порт.
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let local_port = crate::sync::lock(&self.remote_forwards).get(&connected_port).copied();
        // Канал теперь надо подтвердить явно: без accept он отклоняется при сбросе `reply`.
        // Нет маршрута на этот порт - так и отклоняем, а не открываем «в никуда».
        if local_port.is_none() {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        reply.accept().await;
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
    ///
    /// Раньше russh отдавал только идентификатор канала, и запросы приходилось ловить в
    /// колбэке `data`, храня набор «агентских» каналов сбоку. Теперь отдаётся сам канал -
    /// обслуживаем его отдельной задачей, и весь этот учёт больше не нужен.
    async fn server_channel_open_agent_forward(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        let lock = self.agent_lock.clone();
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            let mut channel = channel;
            loop {
                let msg = tokio::select! {
                    m = channel.wait() => m,
                    _ = wait_cancel(cancel.clone()) => break,
                };
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        // Агент один на всё приложение: параллельные запросы к нему
                        // перемешали бы ответы.
                        let _guard = lock.lock().await;
                        match ssh_agent::agent_roundtrip(&data[..]).await {
                            Ok(answer) => {
                                if channel.data(&answer[..]).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                    _ => {}
                }
            }
            let _ = channel.close().await;
        });
        Ok(())
    }
}

/// Base64 открытого ключа сервера - в том же виде, в каком его пишет OpenSSH
/// в `known_hosts`, чтобы отпечатки совпадали со старыми записями.
fn base64_of(key: &russh::keys::PublicKey) -> String {
    use russh::keys::ssh_encoding::Encode;
    let mut blob = Vec::new();
    key.key_data().encode(&mut blob).ok();
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(blob)
}

fn field<'a>(server: &'a Value, key: &str) -> Option<&'a str> {
    server.get(key).and_then(|v| v.as_str())
}

fn port_of(server: &Value) -> u16 {
    server.get("port").and_then(|v| v.as_u64()).unwrap_or(22) as u16
}

fn host_label(server: &Value) -> String {
    let host = field(server, "host").unwrap_or("?");
    let port = port_of(server);
    if port == 22 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

fn auth_rejected(server: &Value) -> crate::error::SereinError {
    crate::error::SereinError::AuthRejected {
        host: host_label(server),
        auth_type: field(server, "authType").unwrap_or("password").to_string(),
    }
}

/// Окно и keepalive под длинные SFTP. `maximum_packet_size` 32 КиБ - как у OpenSSH;
/// SFTP-чанк в `sftp.rs` режется под этот лимит, иначе DATA не влезает в SSH-пакет.
pub(crate) fn ssh_client_config(server: &Value) -> Arc<client::Config> {
    Arc::new(base_client_config(server))
}

fn base_client_config(server: &Value) -> client::Config {
    client::Config {
        // Наборы алгоритмов зависят от профиля: сжатие и режим совместимости со старым железом.
        preferred: crate::ssh_algos::preferred_for(server),
        window_size: 32 * 1024 * 1024,
        maximum_packet_size: 32 * 1024,
        keepalive_interval: Some(std::time::Duration::from_secs(15)),
        keepalive_max: 8,
        // По умолчанию russh оставляет сокету алгоритм Нейгла, и ядро придерживает мелкие
        // пакеты, пока не подтверждён предыдущий. Мелкие пакеты у нас - это нажатия в
        // терминале, движения мыши на рабочем столе и подтверждения окна канала: ровно то,
        // что должно уходить сразу. OpenSSH для интерактивных сессий делает то же самое.
        nodelay: true,
        ..client::Config::default()
    }
}

/// Настройки соединения под рабочий стол.
///
/// Окно здесь маленькое намеренно, и в этом весь смысл отдельного соединения. russh
/// возвращает окно серверу сразу по приходу данных, поэтому окно - это ровно столько,
/// сколько сервер может держать в пути, не дожидаясь нас. При 32 МиБ сервер рабочего
/// стола не чувствует сети вовсе: sshd вычитывает у него всё, что тот успевает
/// нарисовать, и мегабайты кадров встают в очередь перед узким местом VPN. Всё, что
/// идёт следом - нажатия, ответы терминала, замер пинга, - ждёт в той же очереди.
/// С маленьким окном sshd перестаёт читать, сервер RDP упирается в запись и сам
/// пропускает промежуточные кадры: так же, как при прямом подключении mstsc.
///
/// Очередь канала короткая по той же причине: всё, что в ней лежит, - уже устаревшая
/// картинка, которую человек увидит с опозданием.
fn desktop_client_config(server: &Value, window: u32, compress: bool) -> Arc<client::Config> {
    let mut cfg = base_client_config(server);
    cfg.window_size = window.max(cfg.maximum_packet_size);
    cfg.channel_buffer_size = 16;
    if compress {
        cfg.preferred.compression = crate::ssh_algos::compression_first();
    }
    Arc::new(cfg)
}

/// Отдельное SSH-соединение под рабочий стол.
///
/// Общее соединение сессии для рабочего стола не годится: все каналы SSH едут в одном
/// потоке TCP, и поток кадров стоит в нём впереди всего остального. Отдельное соединение
/// - это своя очередь и своё окно, настроенное под картинку, а не под SFTP.
pub struct DesktopLink {
    pub handle: SharedHandle,
    /// Промежуточные хопы цепочки: живут, пока живо само соединение.
    jumps: Vec<client::Handle<ClientHandler>>,
}

impl DesktopLink {
    /// Закрывает соединение и всю цепочку за ним, от ближнего хопа к дальнему.
    pub async fn close(self) {
        let _ = self
            .handle
            .lock()
            .await
            .disconnect(russh::Disconnect::ByApplication, "desktop closed", "")
            .await;
        for j in self.jumps.into_iter().rev() {
            let _ = j
                .disconnect(russh::Disconnect::ByApplication, "desktop closed", "")
                .await;
        }
    }
}

/// Поднимает отдельное соединение под рабочий стол по той же цепочке, что и сессия.
///
/// Ключ хоста принимается только уже подтверждённый: сессия к этому серверу только что
/// прошла проверку, и незнакомый ключ на втором соединении значит подмену, а не новый
/// сервер. Спрашивать о нём здесь некого и незачем.
///
/// Вход только без вопросов: пароль, ключ, агент. Если серверу нужен второй фактор,
/// соединения не будет, и рабочий стол пойдёт общим каналом сессии - вызывающий об этом
/// знает по ошибке.
pub async fn connect_desktop(chain: Vec<Value>, window: u32, compress: bool) -> crate::error::Result<DesktopLink> {
    if chain.is_empty() {
        return Err(crate::error::SereinError::EmptyChain);
    }
    let dummy_ki: KiBridge = Arc::new(Mutex::new(HashMap::new()));
    let remote_forwards: RemoteForwards = Arc::new(Mutex::new(HashMap::new()));
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    // Спросить здесь некого, но срок на шаги машины тот же, что у обычной сессии.
    let pause = HumanPause::default();
    let mut jumps = Vec::new();

    let far = &chain[chain.len() - 1];
    let mut cur = connect_one(
        far,
        desktop_client_config(far, window, compress),
        remote_forwards.clone(),
        cancel_rx.clone(),
        Trust::KnownOnly,
        &pause,
    )
    .await?;
    if !authenticate_within(&mut cur, far, None, &dummy_ki, None, &pause).await? {
        return Err(auth_rejected(far));
    }
    for i in (0..chain.len() - 1).rev() {
        let next = &chain[i];
        let nhost = field(next, "host")
            .ok_or_else(|| crate::error::SereinError::Config("Не задан host промежуточного хоста".into()))?;
        let channel = within_limit(
            &format!("канал до {nhost}"),
            cur.channel_open_direct_tcpip(nhost, port_of(next) as u32, "127.0.0.1", 0),
        )
        .await
        .map_err(|detail| crate::error::SereinError::ProxyJump {
            host: nhost.to_string(),
            detail,
        })?;
        jumps.push(cur);
        let handler = ClientHandler {
            host_id: knownhosts::host_id(nhost, port_of(next)),
            trust: Trust::KnownOnly,
            remote_forwards: remote_forwards.clone(),
            cancel: cancel_rx.clone(),
            agent_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let mut nh = jump_handshake(
            next,
            nhost,
            desktop_client_config(next, window, compress),
            channel,
            handler,
            &pause,
        )
        .await?;
        if !authenticate_within(&mut nh, next, None, &dummy_ki, None, &pause).await? {
            return Err(auth_rejected(next));
        }
        cur = nh;
    }
    Ok(DesktopLink {
        handle: Arc::new(tokio::sync::Mutex::new(cur)),
        jumps,
    })
}

async fn request_ki(app: &AppHandle, ki: &KiBridge, id: &str, prompts: Vec<Value>) -> Vec<String> {
    let (tx, rx) = oneshot::channel();
    crate::sync::lock(ki).insert(id.to_string(), tx);
    let _ = app.emit("session-ki", json!({ "id": id, "prompts": prompts }));
    rx.await.unwrap_or_default()
}

/// Похож ли вопрос сервера на обычный запрос пароля. Новый пароль (истёк срок) и коды
/// второго фактора сохранённым паролем не отвечаются.
fn is_password_prompt(prompt: &str) -> bool {
    let p = prompt.to_lowercase();
    (p.contains("password") || p.contains("пароль"))
        && !["new", "нов", "code", "код", "otp", "token"]
            .iter()
            .any(|w| p.contains(w))
}

/// Аутентификация одного хопа. Для целевого сервера (есть `id`/`ki`) - с поддержкой 2FA.
fn wants_agent_forward(server: &Value) -> bool {
    server.get("agentForward").and_then(|v| v.as_bool()).unwrap_or(false) || field(server, "authType") == Some("agent")
}

async fn authenticate(
    handle: &mut client::Handle<ClientHandler>,
    server: &Value,
    app: Option<&AppHandle>,
    ki: &KiBridge,
    id: Option<&str>,
    pause: &HumanPause,
) -> crate::error::Result<bool> {
    let user = field(server, "username").unwrap_or("root").to_string();
    let auth_type = field(server, "authType").unwrap_or("password");

    match auth_type {
        "key" => {
            let raw = field(server, "privateKeyPath")
                .ok_or_else(|| crate::error::SereinError::Config("Не задан путь к ключу".into()))?;
            let passphrase = field(server, "passphrase").filter(|p| !p.is_empty());
            let path = crate::paths::resolve_identity(raw);
            if !std::path::Path::new(&path).exists() {
                return Err(crate::error::SereinError::Config(crate::paths::missing_key_error(
                    raw, &path,
                )));
            }
            let key = match (load_secret_key(&path, passphrase), app, id) {
                (Ok(k), _, _) => k,
                // Фраза не сохранена - например, это запрещено политикой. Спрашиваем тем же окном,
                // что и ввод от сервера, вместо того чтобы молча не войти.
                (Err(russh::keys::Error::KeyIsEncrypted), Some(app), Some(sid)) if passphrase.is_none() => {
                    let answers = {
                        let _waiting = pause.begin();
                        request_ki(
                            app,
                            ki,
                            sid,
                            vec![json!({ "prompt": format!("Парольная фраза ключа {raw}: "), "echo": false })],
                        )
                        .await
                    };
                    let phrase = answers.into_iter().next().unwrap_or_default();
                    load_secret_key(&path, Some(&phrase))
                        .map_err(|e| crate::error::SereinError::Protocol(e.to_string()))?
                }
                (Err(e), _, _) => return Err(crate::error::SereinError::Protocol(e.to_string())),
            };
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(|e| crate::error::SereinError::Protocol(e.to_string()))?
                .flatten();
            handle
                .authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map(|r| r.success())
                .map_err(|e| crate::error::SereinError::Protocol(e.to_string()))
        }
        "agent" => ssh_agent::authenticate_with_agent(handle, &user, field(server, "agentKey"))
            .await
            .map_err(crate::error::SereinError::Protocol),
        _ => {
            let pass = field(server, "password").unwrap_or("");
            // Отказал ли сервер именно сохранённому паролю. Если метода «password» у сервера
            // нет вовсе, пароль ещё может быть верным - его спросит keyboard-interactive.
            let mut password_refused = false;
            if !pass.is_empty() {
                let r = handle
                    .authenticate_password(&user, pass)
                    .await
                    .map_err(|e| crate::error::SereinError::Protocol(e.to_string()))?;
                if r.success() {
                    return Ok(true);
                }
                if let russh::client::AuthResult::Failure { remaining_methods, .. } = &r {
                    password_refused = remaining_methods.contains(&russh::MethodKind::Password);
                }
            }
            // keyboard-interactive. Человека спрашиваем только в живой сессии, а сохранённым
            // паролем отвечаем и без неё: серверы с `PasswordAuthentication no` и паролем через
            // PAM иначе не пускали туннели, Fleet и задачи вовсе, а в сессии спрашивали пароль
            // при каждом входе, хотя он сохранён.
            let mut saved = (!pass.is_empty() && !password_refused).then_some(pass);
            if id.is_some() || saved.is_some() {
                let mut resp = handle
                    .authenticate_keyboard_interactive_start(&user, None)
                    .await
                    .map_err(|e| crate::error::SereinError::Protocol(e.to_string()))?;
                loop {
                    match resp {
                        KeyboardInteractiveAuthResponse::Success => return Ok(true),
                        KeyboardInteractiveAuthResponse::Failure { .. } => {
                            // Сервер сам пароль не спрашивает (у него только метод «password»), а
                            // сохранённого нет - спрашиваем сами, одним вопросом.
                            if let (Some(app), Some(sid)) = (app.filter(|_| pass.is_empty()), id) {
                                let answers = {
                                    let _waiting = pause.begin();
                                    request_ki(
                                        app,
                                        ki,
                                        sid,
                                        vec![json!({ "prompt": format!("Пароль для {user}: "), "echo": false })],
                                    )
                                    .await
                                };
                                if let Some(p) = answers.into_iter().next().filter(|p| !p.is_empty()) {
                                    return handle
                                        .authenticate_password(&user, &p)
                                        .await
                                        .map(|r| r.success())
                                        .map_err(|e| crate::error::SereinError::Protocol(e.to_string()));
                                }
                            }
                            return Ok(false);
                        }
                        KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                            let password_asked =
                                prompts.len() == 1 && !prompts[0].echo && is_password_prompt(&prompts[0].prompt);
                            let answers = if let Some(p) = saved.filter(|_| password_asked) {
                                // Сохранённым паролем отвечаем один раз: повторный вопрос значит,
                                // что он не подошёл, и дальше решает человек.
                                saved = None;
                                vec![p.to_string()]
                            } else if let (Some(app), Some(sid)) = (app, id) {
                                let pl: Vec<Value> = prompts
                                    .iter()
                                    .map(|p| json!({ "prompt": p.prompt, "echo": p.echo }))
                                    .collect();
                                let _waiting = pause.begin();
                                request_ki(app, ki, sid, pl).await
                            } else if prompts.is_empty() {
                                // Пустой запрос: сервер показывает текст и ждёт пустого ответа.
                                Vec::new()
                            } else {
                                return Err(crate::error::SereinError::Config(
                                    "сервер спрашивает при входе не только пароль, а ответить некому - подключитесь в обычной вкладке".into(),
                                ));
                            };
                            resp = handle
                                .authenticate_keyboard_interactive_respond(answers)
                                .await
                                .map_err(|e| crate::error::SereinError::Protocol(e.to_string()))?;
                        }
                    }
                }
            }
            Ok(false)
        }
    }
}

/// Вход на один хоп с пределом по времени.
///
/// Сервер, принявший соединение и замолчавший на входе, раньше держал подключение сколько
/// угодно: вкладка крутила «подключение» без конца. Ожидание человека - код второго
/// фактора - в срок не входит.
async fn authenticate_within(
    handle: &mut client::Handle<ClientHandler>,
    server: &Value,
    app: Option<&AppHandle>,
    ki: &KiBridge,
    id: Option<&str>,
    pause: &HumanPause,
) -> crate::error::Result<bool> {
    match machine_limit(pause, AUTH_LIMIT, authenticate(handle, server, app, ki, id, pause)).await {
        Ok(r) => r,
        Err(()) => Err(crate::error::SereinError::ConnectFailed {
            host: host_label(server),
            detail: format!("сервер не ответил на вход за {} с", AUTH_LIMIT.as_secs()),
            phase: crate::error::SessionPhase::Auth,
        }),
    }
}

/// Рукопожатие с хопом поверх канала предыдущего. Срок - тот же, что у прямого подключения
/// к нему; вопрос человеку о ключе в срок не входит.
async fn jump_handshake(
    server: &Value,
    host: &str,
    config: Arc<client::Config>,
    channel: Channel<Msg>,
    handler: ClientHandler,
    pause: &HumanPause,
) -> crate::error::Result<client::Handle<ClientHandler>> {
    let (secs, limit) = connect_limit(server);
    let fut = client::connect_stream(config, channel.into_stream(), handler);
    match machine_limit(pause, limit, fut).await {
        Ok(r) => r.map_err(|e| crate::error::SereinError::ProxyJump {
            host: host.to_string(),
            detail: e.to_string(),
        }),
        Err(()) => Err(crate::error::SereinError::ProxyJump {
            host: host.to_string(),
            detail: format!("сервер не ответил за {secs} с"),
        }),
    }
}

async fn connect_one(
    server: &Value,
    config: Arc<client::Config>,
    rf: RemoteForwards,
    cancel: CancelRx,
    trust: Trust,
    pause: &HumanPause,
) -> crate::error::Result<client::Handle<ClientHandler>> {
    let host = field(server, "host").ok_or_else(|| crate::error::SereinError::Config("Не задан host".into()))?;
    let label = host_label(server);
    let handler = ClientHandler {
        host_id: knownhosts::host_id(host, port_of(server)),
        trust,
        remote_forwards: rf,
        cancel,
        agent_lock: Arc::new(tokio::sync::Mutex::new(())),
    };
    let (secs, timeout) = connect_limit(server);

    if let Some(cmd) = field(server, "proxyCommand").map(str::trim).filter(|c| !c.is_empty()) {
        let user = field(server, "username").unwrap_or("root");
        let stream =
            crate::proxycmd::spawn(cmd, host, port_of(server), user).map_err(crate::error::SereinError::Config)?;
        let fut = client::connect_stream(config, stream, handler);
        return match machine_limit(pause, timeout, fut).await {
            Ok(r) => r.map_err(|e| crate::error::SereinError::ConnectFailed {
                host: label.clone(),
                detail: format!("через прокси-команду: {e}"),
                phase: crate::error::SessionPhase::Connect,
            }),
            Err(_) => Err(crate::error::SereinError::ConnectTimeout { host: label, secs }),
        };
    }

    let fut = client::connect(config, (host, port_of(server)), handler);
    match machine_limit(pause, timeout, fut).await {
        Ok(r) => r.map_err(|e| crate::error::SereinError::ConnectFailed {
            host: label.clone(),
            detail: e.to_string(),
            phase: crate::error::SessionPhase::Connect,
        }),
        Err(_) => Err(crate::error::SereinError::ConnectTimeout { host: label, secs }),
    }
}

/// Подключается по цепочке: chain[0] - цель, chain[1..] - jump-хосты (как в Electron).
pub async fn connect_chain(
    app: AppHandle,
    id: String,
    chain: Vec<Value>,
    cols: u32,
    rows: u32,
    ki: KiBridge,
    host_keys: HostKeyBridge,
) -> crate::error::Result<SshSession> {
    if chain.is_empty() {
        return Err(crate::error::SereinError::EmptyChain);
    }
    let target = chain[0].clone();
    let mut jump_handles: Vec<Arc<client::Handle<ClientHandler>>> = Vec::new();
    let remote_forwards: RemoteForwards = Arc::new(Mutex::new(HashMap::new()));
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let alive = Arc::new(AtomicBool::new(true));
    // Вопросы про ключ хоста задаём в UI этой сессии - и для цели, и для каждого jump-хопа.
    let pause = HumanPause::default();
    let ask = Trust::Ask(HostKeyAsk {
        app: app.clone(),
        bridge: host_keys.clone(),
        session_id: id.clone(),
        pause: pause.clone(),
    });

    // Самый дальний хоп (конец цепочки) - прямое подключение.
    let far = &chain[chain.len() - 1];
    let mut handle = connect_one(
        far,
        ssh_client_config(far),
        remote_forwards.clone(),
        cancel_rx.clone(),
        ask.clone(),
        &pause,
    )
    .await?;
    let far_is_target = chain.len() == 1;
    if !authenticate_within(
        &mut handle,
        far,
        Some(&app),
        &ki,
        if far_is_target { Some(id.as_str()) } else { None },
        &pause,
    )
    .await?
    {
        return Err(auth_rejected(far));
    }

    // Идём внутрь к цели: на каждом шаге пробрасываем direct-tcpip и подключаемся поверх.
    let mut cur = handle;
    for i in (0..chain.len() - 1).rev() {
        let next = &chain[i];
        let nhost = field(next, "host")
            .ok_or_else(|| crate::error::SereinError::Config("Не задан host промежуточного хоста".into()))?;
        let channel = within_limit(
            &format!("канал до {nhost}"),
            cur.channel_open_direct_tcpip(nhost, port_of(next) as u32, "127.0.0.1", 0),
        )
        .await
        .map_err(|detail| crate::error::SereinError::ProxyJump {
            host: nhost.to_string(),
            detail,
        })?;
        jump_handles.push(Arc::new(cur));
        let config = ssh_client_config(next);
        let handler = ClientHandler {
            host_id: knownhosts::host_id(nhost, port_of(next)),
            trust: ask.clone(),
            remote_forwards: remote_forwards.clone(),
            cancel: cancel_rx.clone(),
            agent_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let mut nh = jump_handshake(next, nhost, config, channel, handler, &pause).await?;
        let is_target = i == 0;
        if !authenticate_within(
            &mut nh,
            next,
            Some(&app),
            &ki,
            if is_target { Some(id.as_str()) } else { None },
            &pause,
        )
        .await?
        {
            return Err(auth_rejected(next));
        }
        cur = nh;
    }

    let target_label = host_label(&target);
    let shell_err = |detail: String| crate::error::SereinError::ConnectFailed {
        host: target_label.clone(),
        detail,
        phase: crate::error::SessionPhase::Shell,
    };
    let mut channel = within_limit("канал терминала", cur.channel_open_session())
        .await
        .map_err(shell_err)?;
    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .map_err(|e| shell_err(e.to_string()))?;
    if wants_agent_forward(&target) {
        channel
            .agent_forward(true)
            .await
            .map_err(|e| shell_err(e.to_string()))?;
    }
    channel
        .request_shell(true)
        .await
        .map_err(|e| shell_err(e.to_string()))?;

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
                "phase": crate::error::SessionPhase::Shell.as_str(),
                "error": is_drop.then(|| "Соединение разорвано сервером".to_string()),
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
        remote_fs: Arc::new(Mutex::new(crate::remote_fs::SessionFs::new())),
    })
}

/// Подключение без PTY/shell - для exec/SFTP (bench и служебные каналы).
pub async fn connect_client(chain: Vec<Value>) -> crate::error::Result<SharedHandle> {
    if chain.is_empty() {
        return Err(crate::error::SereinError::EmptyChain);
    }
    let dummy_ki: KiBridge = Arc::new(Mutex::new(HashMap::new()));
    let remote_forwards: RemoteForwards = Arc::new(Mutex::new(HashMap::new()));
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    // Спросить здесь некого, но срок на шаги машины тот же, что у обычной сессии.
    let pause = HumanPause::default();
    let far = &chain[chain.len() - 1];
    let mut handle = connect_one(
        far,
        ssh_client_config(far),
        remote_forwards.clone(),
        cancel_rx.clone(),
        Trust::background(),
        &pause,
    )
    .await?;
    if !authenticate_within(&mut handle, far, None, &dummy_ki, None, &pause).await? {
        return Err(auth_rejected(far));
    }
    let mut cur = handle;
    for i in (0..chain.len() - 1).rev() {
        let next = &chain[i];
        let nhost = field(next, "host")
            .ok_or_else(|| crate::error::SereinError::Config("Не задан host промежуточного хоста".into()))?;
        let channel = within_limit(
            &format!("канал до {nhost}"),
            cur.channel_open_direct_tcpip(nhost, port_of(next) as u32, "127.0.0.1", 0),
        )
        .await
        .map_err(|detail| crate::error::SereinError::ProxyJump {
            host: nhost.to_string(),
            detail,
        })?;
        let config = ssh_client_config(next);
        let handler = ClientHandler {
            host_id: knownhosts::host_id(nhost, port_of(next)),
            trust: Trust::background(),
            remote_forwards: remote_forwards.clone(),
            cancel: cancel_rx.clone(),
            agent_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let mut nh = jump_handshake(next, nhost, config, channel, handler, &pause).await?;
        if !authenticate_within(&mut nh, next, None, &dummy_ki, None, &pause).await? {
            return Err(auth_rejected(next));
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
/// `cancel` - оборвать канал (теardown сессии / стоп логов).
pub async fn exec(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    command: &str,
    cancel: Option<CancelRx>,
) -> Result<(i32, String, String), String> {
    exec_with(handle, command, cancel, |_| {}).await
}

/// Итог команды по коду выхода: ноль - успех, иначе текст stderr, а если сервер промолчал -
/// `fallback`. Пустое сообщение об ошибке человеку ничего не скажет, поэтому его не бывает.
pub fn exit_result(code: i32, stderr: &str, fallback: impl FnOnce() -> String) -> Result<(), String> {
    if code == 0 {
        return Ok(());
    }
    let err = stderr.trim();
    Err(if err.is_empty() { fallback() } else { err.to_owned() })
}

/// Итог шага с `sudo -S`: вывод при успехе, иначе понятная причина. Неверный пароль sudo
/// выглядит именно так, и сказать об этом прямо полезнее, чем показать сырой вывод пакетного
/// менеджера.
pub fn sudo_result(code: i32, out: &str, err: &str, what: &str) -> Result<String, String> {
    if code == 0 {
        return Ok(out.to_owned());
    }
    if out.contains("incorrect password") || err.contains("incorrect password") {
        return Err("Пароль sudo не подошёл".into());
    }
    let text = format!("{out}\n{err}");
    let text = text.trim();
    Err(if text.is_empty() {
        format!("{what}: код {code}")
    } else {
        text.to_owned()
    })
}

/// Exec, которому на вход подаётся текст.
///
/// Нужен ровно для одного: пароля `sudo`. Его нельзя подставлять в строку команды - она
/// целиком видна в списке процессов сервера всем, кто там есть. Правильный способ - отдать
/// пароль `sudo -S` через стандартный ввод, и вот он.
///
/// Введённое не логируется и нигде не сохраняется: приходит из формы, уходит в канал.
pub async fn exec_with_input(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    command: &str,
    input: &str,
    cancel: Option<CancelRx>,
) -> Result<(i32, String, String), String> {
    exec_inner(handle, command, Some(input), cancel, |_| {}).await
}

pub async fn exec_with(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    command: &str,
    cancel: Option<CancelRx>,
    on_out: impl FnMut(&[u8]),
) -> Result<(i32, String, String), String> {
    exec_inner(handle, command, None, cancel, on_out).await
}

async fn exec_inner(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    command: &str,
    input: Option<&str>,
    cancel: Option<CancelRx>,
    mut on_out: impl FnMut(&[u8]),
) -> Result<(i32, String, String), String> {
    let mut channel = {
        let h = handle.lock().await;
        h.channel_open_session().await.map_err(|e| e.to_string())?
    };
    channel.exec(true, command).await.map_err(|e| e.to_string())?;
    if let Some(text) = input {
        channel
            .data(text.as_bytes())
            .await
            .map_err(|e| format!("Не удалось передать ввод: {e}"))?;
        // Без EOF читающая сторона будет ждать продолжения: `sudo -S` не начнёт работу,
        // пока не поймёт, что пароль закончился.
        channel.eof().await.map_err(|e| e.to_string())?;
    }
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

/// Лимит по умолчанию для удалённых exec без явной отмены (multihost, ping и т.п.).
pub const DEFAULT_EXEC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Exec с общим таймаутом поверх отмены по каналу.
pub async fn exec_timed(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    command: &str,
    cancel: Option<CancelRx>,
    limit: std::time::Duration,
) -> Result<(i32, String, String), String> {
    match tokio::time::timeout(limit, exec(handle, command, cancel)).await {
        Ok(r) => r,
        Err(_) => Err(crate::error::SereinError::Timeout {
            op: format!("exec «{command}»"),
        }
        .to_string()),
    }
}

/// Отклик SSH-сессии в миллисекундах: полный круг «запусти ничего и ответь».
///
/// Это не ICMP ping и не сетевой RTT: в число входит и работа sshd, и запуск команды на
/// сервере. Называть его пингом в интерфейсе нельзя - человек сравнит с `ping` в консоли
/// и не поймёт разницы.
///
/// `None` - ответа не было: истёк срок, сессия мертва, канал не открылся. Раньше здесь
/// возвращалось время в любом случае, поэтому выключенный сервер показывал бодрые 5000 мс
/// как признак живого соединения.
///
/// Команда `cd .` выбрана нарочно: она есть и в `sh`, и в `cmd.exe`, а нам нужен только
/// круг по сети. Код возврата не важен - важно, что сервер ответил хоть что-то.
pub async fn ping(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>) -> Option<u32> {
    let t = std::time::Instant::now();
    match exec_timed(handle, "cd .", None, std::time::Duration::from_secs(5)).await {
        Ok(_) => Some(t.elapsed().as_millis() as u32),
        Err(_) => None,
    }
}

#[derive(Default)]
pub struct OpHub {
    tx: Mutex<HashMap<String, watch::Sender<bool>>>,
}

impl OpHub {
    pub fn begin(&self, key: &str) -> CancelRx {
        let (tx, rx) = watch::channel(false);
        crate::sync::lock(&self.tx).insert(key.to_string(), tx);
        rx
    }

    pub fn cancel(&self, key: &str) {
        if let Some(tx) = crate::sync::lock(&self.tx).get(key) {
            let _ = tx.send(true);
        }
    }

    pub fn finish(&self, key: &str) {
        crate::sync::lock(&self.tx).remove(key);
    }

    pub fn cancel_prefix(&self, prefix: &str) {
        let guard = crate::sync::lock(&self.tx);
        for (k, tx) in guard.iter() {
            if k.starts_with(prefix) {
                let _ = tx.send(true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn шаг_с_sudo_объясняет_отказ_словами() {
        assert_eq!(sudo_result(0, "ок", "", "установка"), Ok("ок".into()));
        let wrong = "[sudo] password for u: Sorry, try again.\nsudo: 1 incorrect password attempt";
        assert_eq!(
            sudo_result(1, "", wrong, "установка"),
            Err("Пароль sudo не подошёл".into())
        );
        assert_eq!(
            sudo_result(100, "E: нет пакета", "", "установка"),
            Err("E: нет пакета".into())
        );
        assert_eq!(sudo_result(3, " ", "\n", "запуск"), Err("запуск: код 3".into()));
    }

    #[test]
    fn итог_команды_никогда_не_бывает_пустой_ошибкой() {
        assert_eq!(exit_result(0, "шум в stderr", || "запас".into()), Ok(()));
        assert_eq!(
            exit_result(2, "  нет прав\n", || "запас".into()),
            Err("нет прав".into())
        );
        assert_eq!(exit_result(2, " \n", || "код 2".into()), Err("код 2".into()));
    }

    #[test]
    fn лазейка_доверия_ключам_живёт_только_в_отладочной_сборке() {
        // Решение отделено от чтения окружения, поэтому проверяется без подмены переменных.
        assert!(matches!(Trust::from_override(true), Trust::AcceptNewForTests));
        assert!(matches!(Trust::from_override(false), Trust::KnownOnly));
        // А вот сам вопрос «просили ли лазейку» в релизной сборке не задаётся никогда:
        // ветки с именем переменной там нет вовсе.
        if !cfg!(debug_assertions) {
            assert!(!Trust::override_requested(), "в релизной сборке лазейки быть не должно");
            assert!(matches!(Trust::background(), Trust::KnownOnly));
        }
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("рантайм")
    }

    #[test]
    fn рабочий_стол_получает_маленькое_окно_а_сессия_большое() {
        // Окно сессии рассчитано на SFTP. Отдать его рабочему столу значило бы снова
        // пустить мегабайты кадров в очередь впереди терминала и замера пинга.
        let server = serde_json::json!({ "host": "h" });
        let session = ssh_client_config(&server);
        let desk = desktop_client_config(&server, 128 * 1024, true);
        assert_eq!(session.window_size, 32 * 1024 * 1024);
        assert_eq!(desk.window_size, 128 * 1024);
        assert!(desk.channel_buffer_size < session.channel_buffer_size);
        assert!(
            session.nodelay && desk.nodelay,
            "мелкие пакеты не должно придерживать ядро"
        );
        assert_eq!(desk.preferred.compression[0].as_ref(), "zlib");
        assert_ne!(session.preferred.compression[0].as_ref(), "zlib");
    }

    #[test]
    fn окно_не_бывает_меньше_одного_пакета() {
        // Иначе сервер не смог бы отправить ни одного полного пакета и встал бы навсегда.
        let desk = desktop_client_config(&serde_json::json!({}), 1024, false);
        assert_eq!(desk.window_size, desk.maximum_packet_size);
    }

    /// Сторож блокера, из-за которого весь SSH-слой был непокрываем.
    ///
    /// `AppHandle` лежит внутри `ClientHandler`, а его упоминание тянет в бинарь GUI-импорты
    /// wry, которым нужен comctl32 версии 6. Пока манифест не встраивался в тестовый бинарь,
    /// это роняло ВСЕ тесты крейта ещё на загрузке (STATUS_ENTRYPOINT_NOT_FOUND) - не только
    /// новый. Если тест снова начнёт падать, причина будет в `build.rs`, а не в самом тесте.
    #[test]
    fn tauri_types_do_not_break_the_test_binary() {
        let none: Option<tauri::AppHandle> = None;
        assert!(none.is_none());
        assert!(std::mem::size_of::<ClientHandler>() > 0);
    }

    #[test]
    fn port_defaults_to_22_and_survives_junk() {
        assert_eq!(port_of(&json!({})), 22);
        assert_eq!(port_of(&json!({ "port": 2222 })), 2222);
        // Порт строкой (так приезжает из некоторых импортов) - не повод падать.
        assert_eq!(port_of(&json!({ "port": "2222" })), 22);
    }

    #[test]
    fn password_prompts_are_recognised() {
        assert!(is_password_prompt("Password: "));
        assert!(is_password_prompt("(probe@127.0.0.1) Password:"));
        assert!(is_password_prompt("Пароль:"));
        assert!(!is_password_prompt("Verification code: "));
        assert!(!is_password_prompt("New password: "));
        assert!(!is_password_prompt("OTP password: "));
        assert!(!is_password_prompt("Username: "));
    }

    #[test]
    fn agent_forwarding_follows_the_auth_type() {
        // Явный флаг.
        assert!(wants_agent_forward(&json!({ "agentForward": true })));
        // Авторизация агентом подразумевает проброс без отдельной галочки: иначе на
        // втором хопе агента уже нет, и цепочка обрывается посреди пути.
        assert!(wants_agent_forward(&json!({ "authType": "agent" })));
        assert!(!wants_agent_forward(&json!({ "authType": "password" })));
        assert!(!wants_agent_forward(&json!({})));
    }

    #[test]
    fn empty_chain_is_rejected_before_any_network_call() {
        match rt().block_on(connect_client(Vec::new())) {
            Ok(_) => panic!("пустая цепочка не должна подключаться"),
            Err(e) => assert!(e.to_string().contains("Пустая цепочка"), "{e}"),
        }
    }

    #[test]
    fn missing_host_is_a_readable_error() {
        match rt().block_on(connect_client(vec![json!({ "username": "root" })])) {
            Ok(_) => panic!("без host подключаться некуда"),
            Err(e) => assert!(e.to_string().contains("host"), "{e}"),
        }
    }

    #[test]
    fn closed_port_fails_with_an_explanation_not_a_hang() {
        // Порт 1 закрыт на любой машине. Проверяем, что стек доходит до сети и возвращает
        // объяснимую ошибку, а не панику и не бесконечное ожидание.
        let res = rt().block_on(connect_client(vec![
            json!({ "host": "127.0.0.1", "port": 1, "username": "nobody", "connectTimeout": 5 }),
        ]));
        match res {
            Ok(_) => panic!("на закрытый порт подключиться было нельзя"),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("127.0.0.1"), "в ошибке должен быть хост: {msg}");
                assert!(msg.len() > 10, "ошибка должна что-то объяснять: {msg}");
            }
        }
    }

    #[test]
    fn cancel_flag_wakes_the_waiter() {
        let (tx, rx) = watch::channel(false);
        let woke = rt().block_on(async move {
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                let _ = tx.send(true);
            });
            tokio::time::timeout(std::time::Duration::from_secs(2), wait_cancel(rx))
                .await
                .is_ok()
        });
        assert!(woke, "ожидание отмены должно завершаться, а не висеть");
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use std::time::Duration;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn молчащий_сервер_упирается_в_срок() {
        let pause = HumanPause::default();
        let started = std::time::Instant::now();
        let r = runtime().block_on(machine_limit(
            &pause,
            Duration::from_millis(100),
            std::future::pending::<()>(),
        ));
        assert!(r.is_err(), "срок вышел");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn ожидание_человека_не_съедает_срок() {
        // Срок 100 мс, а ответ приходит через 400: всё это время ждали человека.
        let pause = HumanPause::default();
        let r = runtime().block_on(async {
            let p = pause.clone();
            let work = async move {
                let _waiting = p.begin();
                tokio::time::sleep(Duration::from_millis(400)).await;
                7
            };
            machine_limit(&pause, Duration::from_millis(100), work).await
        });
        assert_eq!(r, Ok(7));
    }

    #[test]
    fn после_ответа_человека_срок_снова_идёт() {
        let pause = HumanPause::default();
        let r = runtime().block_on(async {
            let p = pause.clone();
            let work = async move {
                {
                    let _waiting = p.begin();
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                std::future::pending::<()>().await
            };
            machine_limit(&pause, Duration::from_millis(100), work).await
        });
        assert!(
            r.is_err(),
            "сервер замолчал после ответа человека - срок обязан сработать"
        );
    }
}
