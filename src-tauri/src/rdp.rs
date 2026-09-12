//! Рабочий стол по RDP.
//!
//! В отличие от VNC, протокол здесь разбирает не этот модуль, а отдельный процесс
//! `serein-rdp`. Причина не в архитектурных предпочтениях: IronRDP через `picky` прибивает
//! знаком `=` шестнадцать крейтов RustCrypto к release candidate'ам, а SSH-ядро приложения
//! использует те же шестнадцать в стабильных версиях. В одном дереве зависимостей они не
//! сходятся - проверено подменой через `[patch]` и откатом `russh` на версию с той же
//! линией: за одним снятым пином немедленно встаёт следующий.
//!
//! Отсюда устройство. Приложение открывает канал `direct-tcpip` внутри живой SSH-сессии,
//! как для VNC, и подставляет его локальным сокетом на петле. Помощник подключается к
//! этому сокету, ничего не зная ни про SSH, ни про сервер. Наружу по-прежнему ничего не
//! открывается: слушаем только `127.0.0.1` и только на время сеанса.
//!
//! Кадры помощник отдаёт в том же формате, что и `vnc.rs`, поэтому интерфейс рисует их
//! тем же кодом. Поверх формата - длина четырьмя байтами: труба, в отличие от канала
//! Tauri, границ сообщений не хранит.

use std::collections::HashMap;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tauri::ipc::{Channel, InvokeResponseBody};

use crate::deskout::Out;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, ReadBuf};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::ssh::{wait_cancel, CancelRx, SharedHandle};

/// Записывает строку в журнал рабочего стола.
///
/// Именно в файл, а не в стандартный поток ошибок: приложение оконное, консоли у него
/// нет, и всё написанное туда пропадает бесследно. Разбор неполадки без этого журнала
/// упирается в чёрный прямоугольник без единой зацепки - проверено на себе.
///
/// Отказ записи глушится осознанно: журнал - подспорье, и ронять из-за него рабочий
/// стол было бы хуже самой неполадки.
pub(crate) fn log(line: &str) {
    use std::io::Write as _;
    let dir = crate::store::config_dir().join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("rdp.log");
    // Журнал не растёт без конца. Метрики пишутся каждые пять секунд всё время, пока открыт
    // рабочий стол: за сутки это десятки мегабайт в профиле, и никто их не убирает.
    // Предыдущий держим одним файлом - для разбора неполадки хватает, а место конечно.
    if std::fs::metadata(&path).map(|m| m.len() > LOG_MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("rdp.log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {line}", crate::term_out::stamp_utc());
    }
}

/// Насколько отпускаем журнал рабочего стола, прежде чем отложить его в сторону.
const LOG_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Строка от интерфейса в тот же журнал.
///
/// Половина пути кадра проходит в webview, и оттуда в журнал приложения нет дороги.
/// Без этой строчки сторона Rust видит только «кадр отправлен», а что с ним стало
/// дальше - тайна, и разбирать её приходится по снимкам экрана.
pub fn note(line: &str) {
    log(&format!("интерфейс: {line}"));
}

/// Живой сеанс: чем слать ввод, чем остановить и куда сейчас идут кадры.
struct Live {
    input: mpsc::UnboundedSender<String>,
    /// Приёмник кадров. Подменяемый: при откреплении окна сеанс не рвётся, а переезжает.
    out: Out,
    /// Один признак остановки на весь сеанс: мост, ввод, чтение кадров и сам процесс
    /// помощника. Раньше их было два - флаг и очередь команд, - и закрытие зависело от
    /// того, кто первый заметит; неподвижный рабочий стол мог оставить процесс висеть.
    stop: Arc<tokio::sync::watch::Sender<bool>>,
}

/// Сколько ждём помощника, прежде чем снять его силой.
const STOP_GRACE: Duration = Duration::from_secs(2);

/// Сколько ждём, пока помощник подключится к нашему сокету на петле.
const HELPER_CONNECT_WAIT: Duration = Duration::from_secs(15);

/// Длина пропуска к локальному сокету, в байтах шестнадцатеричной записи.
const PASS_LEN: usize = 64;

/// Одноразовый пропуск к локальному сокету.
///
/// Сокет слушает петлю, и постучаться в него может любой процесс этой машины - раньше
/// первый подключившийся получал готовый канал к рабочему столу сервера, открытый нашими
/// правами. Пропуск отдаётся помощнику на стандартный вход (в строке запуска он был бы
/// виден всем) и проверяется здесь до того, как хоть один байт уйдёт на сервер.
fn one_time_pass() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; PASS_LEN / 2];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Читает пропуск и сверяет его. Не сошлось - соединение чужое.
async fn check_pass(sock: &mut tokio::net::TcpStream, expected: &str) -> bool {
    let mut buf = [0u8; PASS_LEN];
    match tokio::time::timeout(Duration::from_secs(5), sock.read_exact(&mut buf)).await {
        Ok(Ok(_)) => {
            // Сравнение постоянного времени здесь не нужно: пропуск одноразовый, живёт
            // секунды и сравнивается ровно один раз.
            buf == expected.as_bytes()
        }
        _ => false,
    }
}

/// Пакет закрытия для интерфейса - тем же форматом, каким его шлёт помощник.
///
/// Нужен, когда помощник ушёл, не успев ничего сказать: упал, снят или оборвалась труба.
/// Без этого пакета панель остаётся в состоянии «подключаюсь» или «работает» навсегда.
fn closed_packet(reason: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(9 + reason.len());
    v.push(9u8);
    v.extend_from_slice(&[0u8; 8]);
    v.extend_from_slice(reason.as_bytes());
    v
}

static SESSIONS: Mutex<Option<HashMap<String, Live>>> = Mutex::new(None);

fn with_sessions<T>(f: impl FnOnce(&mut HashMap<String, Live>) -> T) -> T {
    let mut g = crate::sync::lock(&SESSIONS);
    f(g.get_or_insert_with(HashMap::new))
}

/// Что человек выбрал в форме подключения.
///
/// Умолчания здесь не случайны: внутри SSH-канала до машины рядом полный цвет и всё
/// оформление ничего не стоят, а вот на медленном канале за них платят задержкой.
pub struct Options {
    /// Бит на точку: 32, 24 или 16.
    pub color_depth: u16,
    /// Убрать украшения рабочего стола ради трафика.
    pub economy: bool,
    /// Отдать серверу готовый вход, чтобы он не спрашивал имя и пароль второй раз.
    pub autologon: bool,
    /// Какую полосу объявить серверу: VPN включает его экономичный профиль.
    pub network_profile: NetworkProfile,
}

#[derive(Clone, Copy)]
pub enum NetworkProfile {
    Vpn,
    Lan,
}

impl NetworkProfile {
    fn as_str(self) -> &'static str {
        match self {
            Self::Vpn => "vpn",
            Self::Lan => "lan",
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            color_depth: 16,
            economy: true,
            autologon: true,
            network_profile: NetworkProfile::Vpn,
        }
    }
}

/// Куда подключаться: напрямую или каналом внутри уже живой SSH-сессии.
pub enum Target {
    Tcp {
        host: String,
        port: u16,
    },
    Ssh {
        handle: SharedHandle,
        host: String,
        port: u16,
        /// Своё соединение рабочего стола, если его удалось поднять. Закрывается вместе
        /// с сеансом: переживать его ему незачем.
        link: Option<crate::ssh::DesktopLink>,
    },
}

/// Считает байты, не меняя способ копирования потока. Это объём RDP до упаковки в SSH,
/// а не трафик VPN-интерфейса; второй снимается внешним baseline-скриптом.
struct CountingReader<R> {
    inner: R,
    bytes: Arc<AtomicU64>,
}

impl<R: AsyncRead + Unpin> AsyncRead for CountingReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) {
            self.bytes
                .fetch_add((buf.filled().len() - before) as u64, Ordering::Relaxed);
        }
        result
    }
}

/// Где лежит помощник.
///
/// Рядом с самим приложением, а не в PATH: в PATH может оказаться чужой файл с таким же
/// именем, и запускать его мы не хотим. Если помощника нет - это не поломка, а не до
/// конца собранная поставка, и сказать об этом надо словами.
fn helper_path() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("не найти себя на диске: {e}"))?;
    let dir = exe.parent().ok_or("у приложения нет каталога")?;
    let name = if cfg!(windows) {
        "serein-rdp.exe"
    } else {
        "serein-rdp"
    };
    let p = dir.join(name);
    if p.exists() {
        return Ok(p);
    }
    // При запуске из исходников помощник лежит в своей цели сборки.
    let dev = dir.parent().and_then(|d| d.parent()).map(|root| {
        root.join("rdp-helper")
            .join("target")
            .join("release")
            .join(name)
    });
    match dev {
        Some(d) if d.exists() => Ok(d),
        _ => Err(format!(
            "рядом с приложением нет {name}: поставка собрана не полностью"
        )),
    }
}

/// Открывает сеанс RDP и запускает обмен.
pub async fn open(
    id: String,
    ssh_id: String,
    target: Target,
    user: String,
    password: String,
    domain: Option<String>,
    size: (u16, u16),
    opts: Options,
    on_frame: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let helper = helper_path()?;

    // Слушаем только петлю и только один раз: помощник подключится ровно один, а порт
    // выбирает ядро - фиксированный номер рано или поздно окажется занят.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("не поднять локальный сокет: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("не узнать порт локального сокета: {e}"))?
        .port();

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let stop = Arc::new(stop_tx);
    let pass = one_time_pass();
    let bridge_pass = pass.clone();
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let out = Out::new(on_frame);

    // Мост: то, что помощник пишет в локальный сокет, уходит на сервер, и наоборот.
    let bridge_stop = stop.clone();
    let bridge_rx = stop_rx.clone();
    tokio::spawn(async move {
        // Ждать подключения помощника вечно нечего: он либо пришёл сразу, либо не
        // запустился вовсе, а сокет всё это время открыт на петле.
        let accepted = tokio::select! {
            r = tokio::time::timeout(HELPER_CONNECT_WAIT, listener.accept()) => r,
            _ = wait_cancel(bridge_rx.clone()) => {
                return;
            }
        };
        let Ok(Ok((mut sock, _))) = accepted else {
            log("помощник не подключился к локальному сокету - сеанс не начался");
            let _ = bridge_stop.send(true);
            return;
        };
        sock.set_nodelay(true).ok();
        // Пропуск сверяем до того, как хоть один байт уйдёт на сервер: на петлю может
        // постучаться любой процесс этой машины, а канал открыт нашими правами.
        if !check_pass(&mut sock, &bridge_pass).await {
            log("к локальному сокету постучался не наш помощник - соединение отклонено");
            let _ = bridge_stop.send(true);
            return;
        }
        match target {
            Target::Tcp { host, port } => {
                if let Ok(up) = tokio::net::TcpStream::connect((host.as_str(), port)).await {
                    up.set_nodelay(true).ok();
                    pump(sock, up, bridge_rx.clone()).await;
                }
            }
            Target::Ssh {
                handle,
                host,
                port,
                link,
            } => {
                // Порт открывается со стороны сервера, поэтому «127.0.0.1» здесь - его
                // собственная петля, а не наша. Ради этого всё и затевалось.
                match crate::ssh::open_forward_channel(&handle, &host, port).await {
                    Ok(ch) => pump(sock, ch.into_stream(), bridge_rx.clone()).await,
                    Err(e) => log(&e),
                }
                if let Some(link) = link {
                    link.close().await;
                }
            }
        }
        // Мост кончился - кончился и сеанс: без этого чтение кадров и процесс помощника
        // жили бы дальше, каждый по своим причинам.
        let _ = bridge_stop.send(true);
    });

    let mut cmd = Command::new(&helper);
    // Без этого Windows открывает помощнику собственное чёрное окно поверх приложения:
    // он консольный, а консоль ему не нужна - весь обмен идёт по трубам.
    #[cfg(windows)]
    {
        // У tokio::process::Command свой creation_flags, трейт из std не нужен.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let started = cmd
        .arg("--port")
        .arg(port.to_string())
        .arg("--user")
        .arg(&user)
        .args(
            domain
                .iter()
                .flat_map(|d| ["--domain".to_owned(), d.clone()]),
        )
        .arg("--width")
        .arg(size.0.to_string())
        .arg("--height")
        .arg(size.1.to_string())
        .arg("--color-depth")
        .arg(opts.color_depth.to_string())
        .arg("--network-profile")
        .arg(opts.network_profile.as_str())
        .args(if opts.economy {
            vec!["--economy".to_owned()]
        } else {
            Vec::new()
        })
        .args(if opts.autologon {
            Vec::new()
        } else {
            vec!["--no-autologon".to_owned()]
        })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Раньше здесь стоял `null`, и это было ошибкой: при любой неполадке приложение
        // показывало пустой экран и не могло сказать почему. Теперь читаем.
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    // Помощник не запустился - мост ждать некого, и сокет на петле надо закрыть сразу.
    let mut child = match started {
        Ok(c) => c,
        Err(e) => {
            let _ = stop.send(true);
            return Err(format!("не запустить помощника RDP: {e}"));
        }
    };

    // Пароль уходит первой строкой входа, а не доводом: строка запуска процесса видна
    // в системе всем, кто может смотреть список процессов.
    let mut stdin = match child.stdin.take() {
        Some(s) => s,
        None => {
            let _ = stop.send(true);
            return Err("у помощника нет входа".into());
        }
    };
    // Пароль первой строкой, пропуск второй. Обе - на стандартный вход: строка запуска
    // процесса видна в системе всем, кто может смотреть список процессов.
    if let Err(e) = stdin.write_all(format!("{password}\n{pass}\n").as_bytes()).await {
        let _ = stop.send(true);
        return Err(format!("пароль не ушёл помощнику: {e}"));
    }
    stdin.flush().await.ok();

    // Жалобы помощника уводим в журнал приложения. Своей строкой на сообщение, с
    // пометкой - иначе в общем потоке непонятно, кто говорит.
    if let Some(err) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    log(&format!("помощник: {line}"));
                }
            }
        });
    }

    log(&format!(
        "сеанс {id}: помощник запущен, локальный порт {port}, экран {}x{}",
        size.0, size.1
    ));

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = stop.send(true);
            return Err("у помощника нет вывода".into());
        }
    };
    spawn_pipes(id.clone(), child, stdin, stdout, rx, stop_rx, out.clone());

    crate::deskout::remember(&ssh_id, crate::deskout::Kind::Rdp, &id);
    with_sessions(|m| {
        m.insert(
            id,
            Live {
                input: tx,
                out,
                stop,
            },
        )
    });
    Ok(())
}

/// Гоняет байты между помощником и сервером, пока жив сеанс.
async fn pump<A, B>(a: A, b: B, stop: CancelRx)
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (ar, mut aw) = tokio::io::split(a);
    let (br, mut bw) = tokio::io::split(b);
    let up_bytes = Arc::new(AtomicU64::new(0));
    let down_bytes = Arc::new(AtomicU64::new(0));
    let mut ar = CountingReader {
        inner: ar,
        bytes: up_bytes.clone(),
    };
    let mut br = CountingReader {
        inner: br,
        bytes: down_bytes.clone(),
    };
    let up = async { tokio::io::copy(&mut ar, &mut bw).await };
    let down = async { tokio::io::copy(&mut br, &mut aw).await };
    let stats_stop = stop.clone();
    let stats_up = up_bytes.clone();
    let stats_down = down_bytes.clone();
    let stats = tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(5));
        timer.tick().await;
        let mut previous_up = 0u64;
        let mut previous_down = 0u64;
        loop {
            tokio::select! {
                _ = timer.tick() => {}
                _ = wait_cancel(stats_stop.clone()) => break,
            }
            let current_up = stats_up.load(Ordering::Relaxed);
            let current_down = stats_down.load(Ordering::Relaxed);
            let up_mbps = (current_up - previous_up) as f64 * 8.0 / 5_000_000.0;
            let down_mbps = (current_down - previous_down) as f64 * 8.0 / 5_000_000.0;
            log(&format!(
                "метрики RDP-потока: к серверу {up_mbps:.3} Мбит/с, от сервера {down_mbps:.3} Мбит/с; всего {current_up}/{current_down} Б (до SSH/VPN)"
            ));
            previous_up = current_up;
            previous_down = current_down;
        }
    });
    let started = Instant::now();
    // Кто из двух направлений кончился первым - это и есть причина обрыва, и знать её
    // важно: «к серверу больше не пишут» и «сервер больше не отвечает» - разные беды.
    tokio::select! {
        r = up => match r {
            Ok(n) => log(&format!("помощник закрыл свою сторону, передано {n} Б")),
            Err(e) => log(&format!("обрыв в сторону сервера: {e}")),
        },
        r = down => match r {
            Ok(n) => log(&format!("сервер закрыл соединение, получено {n} Б")),
            Err(e) => log(&format!("обрыв от сервера: {e}")),
        },
        // Закрытие сеанса обрывает перекачку немедленно, а не «когда-нибудь потом»:
        // неподвижный рабочий стол не шлёт ничего, и ждать конца потока можно вечно.
        _ = wait_cancel(stop.clone()) => log("сеанс закрыт - мост остановлен"),
    }
    stats.abort();
    log(&format!(
        "итог RDP-потока за {:.3} с: к серверу {} Б, от сервера {} Б (до SSH/VPN)",
        started.elapsed().as_secs_f64(),
        up_bytes.load(Ordering::Relaxed),
        down_bytes.load(Ordering::Relaxed)
    ));
}

/// Две задачи: кадры от помощника в интерфейс, команды из интерфейса помощнику.
fn spawn_pipes(
    id: String,
    mut child: Child,
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    mut rx: mpsc::UnboundedReceiver<String>,
    stop: CancelRx,
    out: Out,
) {
    // Ввод: строка на команду. Команду выхода пропускаем даже после остановки - именно
    // она и просит помощника уйти по-хорошему.
    let in_stop = stop.clone();
    tokio::spawn(async move {
        loop {
            let line = tokio::select! {
                l = rx.recv() => match l {
                    Some(l) => l,
                    None => break,
                },
                _ = wait_cancel(in_stop.clone()) => {
                    // Дочитываем то, что уже стоит в очереди: там лежит «q».
                    match rx.try_recv() {
                        Ok(l) => l,
                        Err(_) => break,
                    }
                }
            };
            if stdin.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if stdin.flush().await.is_err() {
                break;
            }
        }
    });

    // Кадры: длина четырьмя байтами, потом тело.
    tokio::spawn(async move {
        let mut r = BufReader::new(stdout);
        let mut len = [0u8; 4];
        let mut frames = 0u64;
        let mut frame_bytes = 0u64;
        // Сказал ли помощник сам, почему всё закончилось. Если нет - скажем за него:
        // панель иначе останется в «подключаюсь» или «работает» навсегда.
        let mut said_closed = false;
        let mut last_report = Instant::now();
        let mut last_report_frames = 0u64;
        let mut last_report_bytes = 0u64;
        // Причина остановки называется словами: молчаливо оборвавшийся поток кадров
        // выглядит на экране как чёрный прямоугольник, и отличить «сервер отключился»
        // от «мы сами сломались» по нему невозможно.
        let why = loop {
            let head = tokio::select! {
                r = r.read_exact(&mut len) => r,
                _ = wait_cancel(stop.clone()) => break "сеанс закрыт".to_owned(),
            };
            if let Err(e) = head {
                break format!("помощник больше не отвечает: {e}");
            }
            let n = u32::from_be_bytes(len) as usize;
            // Здравый предел: кадр больше этого означает разъехавшийся протокол, а не
            // огромный экран. Без него битая длина попыталась бы выделить гигабайты.
            if n > 64 * 1024 * 1024 {
                break format!("невозможная длина кадра {n} - протокол разъехался");
            }
            let mut body = vec![0u8; n];
            if let Err(e) = r.read_exact(&mut body).await {
                break format!("кадр оборвался на середине: {e}");
            }
            // Размер экрана запоминаем на лету: окно, которое подхватит сеанс после
            // открепления, начинает с пустого холста и должно узнать его величину ещё
            // до первого кадра от сервера.
            if body.first() == Some(&1) && body.len() >= 9 {
                let w = u16::from_be_bytes([body[5], body[6]]);
                let h = u16::from_be_bytes([body[7], body[8]]);
                crate::deskout::note_size(&id, w, h);
            }
            // Причину закрытия помощник сообщает пакетом, а не в поток ошибок: её видит
            // интерфейс. В журнал её надо положить отдельно - иначе там остаётся только
            // «помощник больше не отвечает», а что именно сломалось, знает один экран.
            if body.first() == Some(&9) {
                said_closed = true;
                if body.len() > 9 {
                    log(&format!("помощник закрыл сеанс: {}", String::from_utf8_lossy(&body[9..])));
                }
            }
            if out.send(InvokeResponseBody::Raw(body)).is_err() {
                break "интерфейс больше не слушает".to_owned();
            }
            frames += 1;
            frame_bytes += n as u64;
            if last_report.elapsed() >= Duration::from_secs(5) {
                let seconds = last_report.elapsed().as_secs_f64();
                let interval_frames = frames - last_report_frames;
                let interval_bytes = frame_bytes - last_report_bytes;
                log(&format!(
                    "метрики IPC: {frames} кадров, {frame_bytes} Б суммарно; за интервал {:.2} кадр/с, {:.3} МиБ/с",
                    interval_frames as f64 / seconds,
                    interval_bytes as f64 / 1024.0 / 1024.0 / seconds
                ));
                last_report = Instant::now();
                last_report_frames = frames;
                last_report_bytes = frame_bytes;
            }
        };
        log(&format!(
            "поток кадров окончен: {why}; всего кадров {frames}, {frame_bytes} Б"
        ));
        // Причина закрытия обязана дойти до панели, даже если помощник не успел её назвать.
        if !said_closed {
            let _ = out.send(InvokeResponseBody::Raw(closed_packet(&why)));
        }
        // Помощник мог уйти сам, мог получить «q» и заканчивать, а мог и застрять. Даём
        // ему короткий срок, потом снимаем: висящий процесс с открытым каналом внутри
        // SSH-сессии - это не «почти закрыто», это утечка.
        if tokio::time::timeout(STOP_GRACE, child.wait()).await.is_err() {
            log("помощник не ушёл за 2 с - снимаем");
            let _ = child.kill().await;
        }
        crate::deskout::forget(&id);
        with_sessions(|m| m.remove(&id));
    });
}

/// Отправляет команду в сеанс. Нет такого - молча ничего: сеанс мог закрыться.
fn send(id: &str, line: String) {
    with_sessions(|m| {
        if let Some(s) = m.get(id) {
            let _ = s.input.send(line);
        }
    });
}

pub fn pointer(id: &str, x: u16, y: u16, buttons: u8) {
    send(id, format!("p {x} {y} {buttons}\n"));
}

pub fn key(id: &str, code: u16, down: bool) {
    send(id, format!("k {code} {}\n", u8::from(down)));
}

pub fn wheel(id: &str, vertical: bool, delta: i16) {
    if delta != 0 {
        send(id, format!("w {} {delta}\n", u8::from(vertical)));
    }
}

/// Защищённая последовательность отправляется одним элементом очереди, чтобы между
/// нажатиями не вклинилось движение мыши или другая клавиша из интерфейса.
pub fn secure_attention(id: &str) {
    send(
        id,
        "k 29 1\nk 56 1\nk 57427 1\nk 57427 0\nk 56 0\nk 29 0\n".to_owned(),
    );
}

/// Переводит выдачу кадров в другое окно и просит перерисовать экран целиком.
///
/// Неподвижный рабочий стол не шлёт ничего часами, поэтому без просьбы о полном кадре
/// новое окно осталось бы чёрным до первого движения на сервере.
pub fn attach(id: &str, ch: Channel<InvokeResponseBody>) -> Result<(), String> {
    let found = with_sessions(|m| {
        m.get(id).map(|s| {
            s.out.set(ch);
            s.input.send("f\n".to_owned()).ok();
        })
    });
    found.ok_or_else(|| "Этот рабочий стол уже закрыт".to_owned())
}

/// Просит сервер сменить размер рабочего стола.
///
/// Не переподключение: размер меняется прямо в живом сеансе, по отдельному каналу
/// управления экраном. Сервер, который его не поддержал, просьбу просто не заметит - и
/// картинка останется вписанной в окно.
pub fn resize(id: &str, w: u16, h: u16) {
    send(id, format!("s {w} {h}\n"));
}

pub fn close(id: &str) {
    crate::deskout::forget(id);
    with_sessions(|m| {
        if let Some(s) = m.remove(id) {
            // Сначала просьба уйти по-хорошему, потом признак остановки. Порядок важен:
            // задача ввода пропускает «q» и после остановки, а вот наоборот помощник
            // остался бы ждать кадров от неподвижного стола сколько угодно.
            let _ = s.input.send("q\n".to_owned());
            let _ = s.stop.send(true);
        }
    });
}

/// Закрывает все сеансы: без этого RDP пережил бы собственный туннель.
pub fn close_all() {
    with_sessions(|m| {
        for (id, s) in m.drain() {
            crate::deskout::forget(&id);
            let _ = s.input.send("q\n".to_owned());
            let _ = s.stop.send(true);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn команды_собираются_в_строки_которые_понимает_помощник() {
        // Формат разбирает `rdp-helper/src/proto.rs`, и расхождение здесь означало бы
        // молча проглоченный ввод: помощник просто не узнаёт команду и пропускает её.
        assert_eq!(format!("p {} {} {}\n", 10, 20, 1), "p 10 20 1\n");
        assert_eq!(format!("k {} {}\n", 65, u8::from(true)), "k 65 1\n");
        assert_eq!(format!("k {} {}\n", 65, u8::from(false)), "k 65 0\n");
        assert_eq!(format!("w {} {}\n", u8::from(true), -120), "w 1 -120\n");
    }

    #[test]
    fn чужое_соединение_к_локальному_сокету_отклоняется() {
        // Сокет слушает петлю, и постучаться в него может любой процесс этой машины. Раньше
        // первый подключившийся получал готовый канал к рабочему столу сервера, открытый
        // нашими правами.
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let pass = one_time_pass();
            assert_eq!(pass.len(), PASS_LEN, "пропуск нужной длины");
            assert_ne!(pass, one_time_pass(), "каждый сеанс - свой пропуск");

            let l = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let addr = l.local_addr().unwrap();

            // Свой: присылает пропуск и проходит.
            let ожидаемый = pass.clone();
            let гость = tokio::spawn(async move {
                let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
                s.write_all(ожидаемый.as_bytes()).await.unwrap();
                // Держим сокет открытым, пока проверяют.
                tokio::time::sleep(Duration::from_millis(200)).await;
            });
            let (mut sock, _) = l.accept().await.unwrap();
            assert!(check_pass(&mut sock, &pass).await, "свой помощник обязан пройти");
            гость.await.unwrap();

            // Чужой: пропуск не тот - не проходит.
            let чужой = tokio::spawn(async move {
                let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
                s.write_all(&[b'0'; PASS_LEN]).await.unwrap();
                tokio::time::sleep(Duration::from_millis(200)).await;
            });
            let (mut sock, _) = l.accept().await.unwrap();
            assert!(!check_pass(&mut sock, &pass).await, "чужому здесь делать нечего");
            чужой.await.unwrap();
        });
    }

    #[test]
    fn пакет_закрытия_собирается_так_же_как_у_помощника() {
        // Этим пакетом панель узнаёт, что всё кончилось, и выходит из «подключаюсь».
        // Разбирает его тот же код, что и кадры VNC, поэтому заголовок обязан совпадать.
        let p = closed_packet("сервер ушёл");
        assert_eq!(p[0], 9, "вид пакета - закрытие");
        assert_eq!(&p[1..9], &[0u8; 8], "координаты и размеры здесь ничего не значат");
        assert_eq!(String::from_utf8_lossy(&p[9..]), "сервер ушёл");
    }

    #[test]
    fn закрытие_несуществующего_сеанса_не_роняет() {
        // Интерфейс закрывает панель и по своей воле, и по приходу пакета закрытия -
        // второй вызов на тот же идентификатор обязан быть безобидным.
        close("нет такого");
        close("нет такого");
    }
}
