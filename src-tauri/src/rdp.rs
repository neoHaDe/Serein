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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tauri::ipc::{Channel, InvokeResponseBody};

use crate::deskout::Out;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, ReadBuf};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::ssh::SharedHandle;

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
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("rdp.log"))
    {
        let _ = writeln!(f, "{} {line}", crate::term_out::stamp_utc());
    }
}

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

    let alive = Arc::new(AtomicBool::new(true));
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let out = Out::new(on_frame);

    // Мост: то, что помощник пишет в локальный сокет, уходит на сервер, и наоборот.
    let bridge_alive = alive.clone();
    tokio::spawn(async move {
        let Ok((sock, _)) = listener.accept().await else {
            return;
        };
        sock.set_nodelay(true).ok();
        match target {
            Target::Tcp { host, port } => {
                if let Ok(up) = tokio::net::TcpStream::connect((host.as_str(), port)).await {
                    up.set_nodelay(true).ok();
                    pump(sock, up, bridge_alive).await;
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
                let ch = {
                    let g = handle.lock().await;
                    g.channel_open_direct_tcpip(host.as_str(), port as u32, "127.0.0.1", 0)
                        .await
                };
                match ch {
                    Ok(ch) => pump(sock, ch.into_stream(), bridge_alive).await,
                    Err(e) => log(&format!("канал до {host}:{port} не открылся: {e}")),
                }
                if let Some(link) = link {
                    link.close().await;
                }
            }
        }
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
    let mut child = cmd
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
        .spawn()
        .map_err(|e| format!("не запустить помощника RDP: {e}"))?;

    // Пароль уходит первой строкой входа, а не доводом: строка запуска процесса видна
    // в системе всем, кто может смотреть список процессов.
    let mut stdin = child.stdin.take().ok_or("у помощника нет входа")?;
    stdin
        .write_all(format!("{password}\n").as_bytes())
        .await
        .map_err(|e| format!("пароль не ушёл помощнику: {e}"))?;
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

    let stdout = child.stdout.take().ok_or("у помощника нет вывода")?;
    spawn_pipes(
        id.clone(),
        child,
        stdin,
        stdout,
        rx,
        alive.clone(),
        out.clone(),
    );

    crate::deskout::remember(&ssh_id, crate::deskout::Kind::Rdp, &id);
    with_sessions(|m| m.insert(id, Live { input: tx, out }));
    Ok(())
}

/// Гоняет байты между помощником и сервером, пока жив сеанс.
async fn pump<A, B>(a: A, b: B, alive: Arc<AtomicBool>)
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
    let stats_alive = alive.clone();
    let stats_up = up_bytes.clone();
    let stats_down = down_bytes.clone();
    let stats = tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(5));
        timer.tick().await;
        let mut previous_up = 0u64;
        let mut previous_down = 0u64;
        while stats_alive.load(Ordering::Relaxed) {
            timer.tick().await;
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
    }
    alive.store(false, Ordering::Relaxed);
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
    alive: Arc<AtomicBool>,
    out: Out,
) {
    // Ввод: строка на команду.
    let in_alive = alive.clone();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if !in_alive.load(Ordering::Relaxed) {
                break;
            }
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
        let mut last_report = Instant::now();
        let mut last_report_frames = 0u64;
        let mut last_report_bytes = 0u64;
        // Причина остановки называется словами: молчаливо оборвавшийся поток кадров
        // выглядит на экране как чёрный прямоугольник, и отличить «сервер отключился»
        // от «мы сами сломались» по нему невозможно.
        let why = loop {
            if !alive.load(Ordering::Relaxed) {
                break "сеанс закрыт".to_owned();
            }
            if let Err(e) = r.read_exact(&mut len).await {
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
            if body.first() == Some(&9) && body.len() > 9 {
                log(&format!("помощник закрыл сеанс: {}", String::from_utf8_lossy(&body[9..])));
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
        alive.store(false, Ordering::Relaxed);
        // Помощник мог уже уйти сам; если нет - не оставляем его висеть.
        let _ = child.kill().await;
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
            // Не гасим `alive` здесь: писатель проверяет его перед записью, и прежний
            // порядок выбрасывал `q` прямо перед отправкой. Помощник получает команду,
            // закрывает сокет, после чего обе задачи завершаются естественно.
            let _ = s.input.send("q\n".to_owned());
        }
    });
}

/// Закрывает все сеансы: без этого RDP пережил бы собственный туннель.
pub fn close_all() {
    with_sessions(|m| {
        for (id, s) in m.drain() {
            crate::deskout::forget(&id);
            let _ = s.input.send("q\n".to_owned());
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
    fn закрытие_несуществующего_сеанса_не_роняет() {
        // Интерфейс закрывает панель и по своей воле, и по приходу пакета закрытия -
        // второй вызов на тот же идентификатор обязан быть безобидным.
        close("нет такого");
        close("нет такого");
    }
}
