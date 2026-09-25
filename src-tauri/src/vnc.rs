//! VNC (RFB) - удалённый рабочий стол в той же вкладке, что и терминал.
//!
//! Устроено вокруг одного решения: **пиксели не ходят через JSON**. Кадр 1920×1080 в BGRA -
//! это 8 МБ; в JSON-массиве чисел он превращается в десятки мегабайт текста, и приложение
//! умирает на первом же обновлении экрана. Поэтому кадры уходят во фронтенд сырыми байтами
//! через `Channel<InvokeResponseBody::Raw>`, а формат пакета описан ниже и разбирается на той
//! стороне через `DataView`.
//!
//! Второе решение - **поток задаётся снаружи**. `VncConnector` принимает любой
//! `AsyncRead + AsyncWrite`, поэтому подключение может идти либо напрямую по TCP, либо
//! каналом уже открытой SSH-сессии. Второй путь и есть то, ради чего это делается своим
//! клиентом, а не запуском чужого: VNC почти всегда слушает `127.0.0.1` на сервере и наружу
//! не смотрит - правильно и есть ходить к нему через SSH, не открывая порт в мир.

use crate::ssh::SharedHandle;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::mpsc;
pub use vnc::X11Event;
use vnc::{PixelFormat, VncClient, VncConnector, VncEncoding, VncError, VncEvent};

/// Тип пакета в первом байте. Значения дублируются во фронтенде - держать их синхронно.
///
/// Причина обрыва приходит текстом внутри CLOSED, а в полях `x` и `y` этого пакета лежат
/// два признака: «дело в пароле» и «сервер временно закрыл доступ». Признаки отдельные, а
/// не разбор текста: сообщение приходит от сервера, оно на его языке и в его формулировке,
/// и строить на нём логику - значит ломаться от чужой правки.
///
/// Различать эти два случая обязательно: советы у них противоположные. При неверном
/// пароле надо ввести другой, при блокировке любой пароль отвергается не глядя, и
/// предлагать ввод значит подсказывать то, что блокировку продлевает.
mod kind {
    pub const RESIZE: u8 = 1;
    pub const RAW: u8 = 2;
    pub const JPEG: u8 = 3;
    pub const COPY: u8 = 4;
    pub const CURSOR: u8 = 5;
    pub const BELL: u8 = 6;
    pub const TEXT: u8 = 7;
    pub const CLOSED: u8 = 9;
}

/// Заголовок прямоугольника: тип и четыре координаты, по два байта на каждую.
fn header(k: u8, x: u16, y: u16, w: u16, h: u16) -> Vec<u8> {
    let mut v = Vec::with_capacity(9);
    v.push(k);
    for n in [x, y, w, h] {
        v.extend_from_slice(&n.to_be_bytes());
    }
    v
}

fn packet(k: u8, x: u16, y: u16, w: u16, h: u16, body: &[u8]) -> Vec<u8> {
    let mut v = header(k, x, y, w, h);
    v.extend_from_slice(body);
    v
}

/// Пиксели приходят BGRA (мы сами просим этот формат), а canvas ждёт RGBA.
///
/// Перестановка на месте, без выделения второго буфера: на каждый кадр это мегабайты, и
/// лишняя копия здесь стоит дороже, чем выглядит.
fn bgra_to_rgba(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

struct Live {
    input: mpsc::UnboundedSender<X11Event>,
    alive: Arc<AtomicBool>,
    /// Приёмник кадров. Подменяемый: при откреплении окна сеанс не рвётся, а переезжает.
    out: crate::deskout::Out,
}

static SESSIONS: Mutex<Option<HashMap<String, Live>>> = Mutex::new(None);

fn with_sessions<T>(f: impl FnOnce(&mut HashMap<String, Live>) -> T) -> T {
    let mut g = crate::sync::lock(&SESSIONS);
    f(g.get_or_insert_with(HashMap::new))
}

/// Ошибка открытия сессии - с признаком «дело в пароле».
///
/// Рукопожатие падает до того, как начнётся цикл кадров, поэтому отказ по паролю приходит
/// сюда, а не пакетом закрытия. Без отдельного поля интерфейсу пришлось бы разбирать текст
/// сообщения, и форма ввода пароля не показалась бы там, где она нужнее всего.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenError {
    pub message: String,
    pub needs_password: bool,
    /// Сервер временно закрыл доступ после неудачных попыток.
    ///
    /// Отдельно от `needs_password`, потому что это противоположный совет. При неверном
    /// пароле надо ввести другой; при блокировке любой пароль отвергается не глядя, и
    /// вводить что-либо бессмысленно, пока она не снята.
    pub blacklisted: bool,
}

impl From<&VncError> for OpenError {
    fn from(e: &VncError) -> Self {
        let blacklisted = is_blacklisted(e);
        Self {
            message: vnc_err(e),
            // При блокировке форму пароля не показываем: она подсказывала бы сделать
            // ровно то, что продлевает блокировку.
            needs_password: !blacklisted && is_auth_failure(e),
            blacklisted,
        }
    }
}

impl From<String> for OpenError {
    fn from(message: String) -> Self {
        Self {
            message,
            needs_password: false,
            blacklisted: false,
        }
    }
}

/// Куда подключаться: напрямую или каналом внутри уже живой SSH-сессии.
pub enum Target {
    Tcp {
        host: String,
        port: u16,
    },
    /// Через SSH: канал `direct-tcpip` до `host:port` со стороны сервера.
    ///
    /// `link` - своё соединение рабочего стола, если его удалось поднять; `handle` тогда
    /// указывает на него. Закрывается вместе с сеансом.
    Ssh {
        handle: SharedHandle,
        host: String,
        port: u16,
        link: Option<crate::ssh::DesktopLink>,
    },
}

/// Открывает VNC-сессию и запускает цикл событий.
///
/// Возвращает идентификатор, по которому потом идут ввод и закрытие.
pub async fn open(
    id: String,
    ssh_id: String,
    target: Target,
    password: Option<String>,
    on_frame: Channel<InvokeResponseBody>,
) -> Result<(), OpenError> {
    let (tx, rx) = mpsc::unbounded_channel::<X11Event>();
    let alive = Arc::new(AtomicBool::new(true));
    let out = crate::deskout::Out::new(on_frame);

    match target {
        Target::Tcp { host, port } => {
            let sock = tokio::net::TcpStream::connect((host.as_str(), port))
                .await
                .map_err(|e| OpenError::from(format!("Не удалось подключиться к {host}:{port}: {e}")))?;
            sock.set_nodelay(true).ok();
            spawn_loop(id.clone(), sock, password, rx, alive.clone(), out.clone(), None).await?;
        }
        Target::Ssh {
            handle,
            host,
            port,
            link,
        } => {
            // Порт открывается со стороны сервера, поэтому «127.0.0.1» здесь - это его
            // собственный loopback, а не наш. Ради этого всё и затевалось.
            let ch = match crate::ssh::open_forward_channel(&handle, &host, port).await {
                Ok(ch) => ch,
                Err(e) => {
                    if let Some(link) = link {
                        link.close().await;
                    }
                    return Err(OpenError::from(e));
                }
            };
            spawn_loop(
                id.clone(),
                ch.into_stream(),
                password,
                rx,
                alive.clone(),
                out.clone(),
                link,
            )
            .await?;
        }
    }

    crate::deskout::remember(&ssh_id, crate::deskout::Kind::Vnc, &id);
    with_sessions(|m| m.insert(id, Live { input: tx, alive, out }));
    Ok(())
}

/// Рукопожатие RFB вплоть до готового клиента.
///
/// Пароль не задан - серверу не отвечаем вовсе. Библиотека спрашивает пароль после того,
/// как сервер прислал вызов, и пустой ответ он засчитал бы как неудачную попытку входа: а
/// после нескольких таких TigerVNC закрывает доступ. К тому же окно сразу писало «Неверный
/// пароль VNC» человеку, который ещё ничего не вводил, - первое подключение идёт без пароля.
async fn handshake<S>(stream: S, password: Option<String>) -> Result<VncClient, VncError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + Sync + 'static,
{
    let secret = password.filter(|p| !p.is_empty());
    VncConnector::new(stream)
        .set_auth_method(async move { secret.ok_or(VncError::NoPassword) })
        .add_encoding(VncEncoding::Tight)
        .add_encoding(VncEncoding::Zrle)
        .add_encoding(VncEncoding::CopyRect)
        .add_encoding(VncEncoding::Raw)
        .add_encoding(VncEncoding::CursorPseudo)
        .add_encoding(VncEncoding::DesktopSizePseudo)
        .set_pixel_format(PixelFormat::bgra())
        .build()?
        .try_start()
        .await?
        .finish()
}

async fn spawn_loop<S>(
    id: String,
    stream: S,
    password: Option<String>,
    mut rx: mpsc::UnboundedReceiver<X11Event>,
    alive: Arc<AtomicBool>,
    out: crate::deskout::Out,
    link: Option<crate::ssh::DesktopLink>,
) -> Result<(), OpenError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + Sync + 'static,
{
    let started = handshake(stream, password).await.map_err(|e| OpenError::from(&e));
    // Не вошли - своё соединение рабочего стола больше не нужно, а висеть оно будет,
    // пока его не закроют явно.
    let client = match started {
        Ok(c) => c,
        Err(e) => {
            if let Some(link) = link {
                link.close().await;
            }
            return Err(e);
        }
    };

    let client = Arc::new(client);

    // Ввод и события идут двумя задачами. Тонкость в библиотеке: `input` и
    // `recv_event` берут один и тот же внутренний замок, а `recv_event` держит его всё
    // время ожидания следующего кадра. Читать экран блокирующим `recv_event` значит
    // намертво перекрыть ввод: нажатия доходят до канала и встают в очередь навсегда.
    // Поэтому ниже опрос через `poll_event`, который замок сразу отпускает.
    let writer = client.clone();
    let writer_alive = alive.clone();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if !writer_alive.load(Ordering::Relaxed) {
                break;
            }
            if writer.input(ev).await.is_err() {
                break;
            }
        }
    });

    let reader_alive = alive.clone();
    tokio::spawn(async move {
        let end = event_loop(&client, &reader_alive, &id, &out).await;
        reader_alive.store(false, Ordering::Relaxed);
        let _ = client.close().await;
        if let Some(link) = link {
            link.close().await;
        }
        let (text, auth, blocked) = end.unwrap_or_default();
        let _ = out.send(InvokeResponseBody::Raw(packet(
            kind::CLOSED,
            u16::from(auth),
            u16::from(blocked),
            0,
            0,
            text.as_bytes(),
        )));
        crate::deskout::forget(&id);
        with_sessions(|m| m.remove(&id));
    });

    Ok(())
}

/// Отказ именно из-за пароля: сервер требует его или не принял.
///
/// Часть таких отказов библиотека отдаёт как `General` с текстом сервера - отсюда проверка
/// по подстроке. Она живёт здесь, в одном месте, а наружу уходит уже флагом.
/// Отказ ли это из-за временной блокировки адреса.
///
/// Формулировка сервера проверена на живом TigerVNC: заблокированному клиенту он
/// отвечает «Too many security failures» и разрывает связь до выбора способа входа.
fn is_blacklisted(e: &VncError) -> bool {
    let text = match e {
        VncError::General(m) => m.to_lowercase(),
        _ => return false,
    };
    text.contains("too many") || text.contains("security failures")
}

fn is_auth_failure(e: &VncError) -> bool {
    match e {
        VncError::WrongPassword | VncError::NoPassword => true,
        VncError::General(m) => {
            let m = m.to_lowercase();
            m.contains("password") || m.contains("auth")
        }
        _ => false,
    }
}

/// Возвращает причину и признак «дело в пароле», если сессия закончилась ошибкой.
///
/// Блокировка сюда тоже попадает: сервер закрывает связь, а не отказывает в пароле.
async fn event_loop(
    client: &vnc::VncClient,
    alive: &AtomicBool,
    id: &str,
    out: &crate::deskout::Out,
) -> Option<(String, bool, bool)> {
    // В RFB сервер не транслирует экран сам по себе: он присылает изменения **в ответ на
    // запрос** и после этого снова молчит. Одного запроса при подключении хватает ровно на
    // одну картинку - дальше экран замирает, хотя ввод доходит и на сервере всё меняется.
    // Поэтому запрос повторяется постоянно, примерно тридцать раз в секунду.
    let mut asked = tokio::time::Instant::now();
    if client.input(X11Event::Refresh).await.is_err() {
        return Some(("Не удалось запросить кадр".into(), false, false));
    }
    // Пауза между пустыми опросами. Достаточно мала, чтобы не съедать кадры на глаз, и
    // достаточно велика, чтобы не крутить процессор впустую на простое.
    let idle = std::time::Duration::from_millis(4);
    let frame = std::time::Duration::from_millis(33);
    while alive.load(Ordering::Relaxed) {
        if asked.elapsed() >= frame {
            asked = tokio::time::Instant::now();
            // Запрос инкрементальный: сервер пришлёт только изменившиеся области, а если
            // не изменилось ничего - не пришлёт ничего и ждать не заставит.
            if client.input(X11Event::Refresh).await.is_err() {
                return Some(("Соединение с рабочим столом потеряно".into(), false, false));
            }
        }
        let ev = match client.poll_event().await {
            Ok(Some(ev)) => ev,
            Ok(None) => {
                tokio::time::sleep(idle).await;
                continue;
            }
            Err(e) => return Some((vnc_err(&e), is_auth_failure(&e), is_blacklisted(&e))),
        };
        let msg = match ev {
            VncEvent::SetResolution(screen) => {
                // Размер запоминаем: окно, которое подхватит сеанс после открепления,
                // начинает с пустого холста и должно узнать его величину заранее.
                crate::deskout::note_size(id, screen.width, screen.height);
                packet(kind::RESIZE, 0, 0, screen.width, screen.height, &[])
            }
            VncEvent::RawImage(rect, data) => {
                let mut px = data.to_vec();
                bgra_to_rgba(&mut px);
                packet(kind::RAW, rect.x, rect.y, rect.width, rect.height, &px)
            }
            VncEvent::JpegImage(rect, data) => {
                // JPEG отдаём как есть: браузер декодирует его сам и быстрее, чем мы,
                // а декодер в Rust - это ещё одна зависимость ради того же результата.
                packet(kind::JPEG, rect.x, rect.y, rect.width, rect.height, &data)
            }
            VncEvent::Copy(dst, src) => {
                let mut v = header(kind::COPY, dst.x, dst.y, dst.width, dst.height);
                v.extend_from_slice(&src.x.to_be_bytes());
                v.extend_from_slice(&src.y.to_be_bytes());
                v
            }
            VncEvent::SetCursor(rect, data) => {
                let mut px = data.to_vec();
                bgra_to_rgba(&mut px);
                packet(kind::CURSOR, rect.x, rect.y, rect.width, rect.height, &px)
            }
            VncEvent::Bell => packet(kind::BELL, 0, 0, 0, 0, &[]),
            VncEvent::Text(t) => packet(kind::TEXT, 0, 0, 0, 0, t.as_bytes()),
            VncEvent::Error(e) => {
                let auth = e.to_lowercase().contains("password") || e.to_lowercase().contains("auth");
                let err = VncError::General(e);
                let blocked = is_blacklisted(&err);
                // При блокировке форму пароля не показываем: она подсказала бы сделать
                // ровно то, что блокировку продлевает.
                return Some((vnc_err(&err), auth && !blocked, blocked));
            }
            // Формат пикселей мы задали сами, повторять его фронтенду незачем.
            VncEvent::SetPixelFormat(_) => continue,
            _ => continue,
        };
        if !out.send(InvokeResponseBody::Raw(msg)) {
            // Окно закрыли - дальше рисовать некому.
            return None;
        }
    }
    None
}

/// Текст ошибки по-русски: сообщения библиотеки английские и местами с опечатками, а это
/// то, что увидит пользователь в окне вместо экрана.
fn vnc_err(e: &VncError) -> String {
    match e {
        VncError::NoPassword => "Рабочий стол защищён паролем VNC - введите его".into(),
        VncError::WrongPassword => "Неверный пароль VNC".into(),
        VncError::InvalidSecurityTyep(t) => {
            format!("Сервер предлагает способ входа {t}, который не поддерживается")
        }
        VncError::ConnectError => "Не удалось согласовать соединение с сервером".into(),
        VncError::ClientNotRunning => "Сессия VNC уже закрыта".into(),
        VncError::IoError(io) => format!("Обрыв связи: {io}"),
        // Сообщения от сервера приходят сюда как есть, по-английски. Самое частое - отказ
        // по паролю, и его пользователь должен прочитать на своём языке. Формулировка у
        // серверов разная: TigerVNC говорит «Authentication failed», x11vnc - «password
        // check failed», поэтому проверяются оба слова.
        // Блокировку проверяем раньше пароля: её текст тоже про неудачный вход, и без
        // этого порядка она читалась бы как «неверный пароль», то есть как приглашение
        // попробовать ещё раз - ровно то, что блокировку и продлевает.
        VncError::General(_) if is_blacklisted(e) => "Сервер временно закрыл доступ после неудачных попыток входа. \
             Правильный пароль сейчас тоже не примут: снимается перезапуском службы \
             VNC на сервере либо ожиданием."
            .into(),
        VncError::General(m)
            if {
                let m = m.to_lowercase();
                m.contains("password") || m.contains("authentication")
            } =>
        {
            "Неверный пароль VNC".into()
        }
        other => other.to_string(),
    }
}

/// Ввод от пользователя. Тихо игнорируется, если сессии уже нет.
pub fn input(id: &str, event: X11Event) {
    with_sessions(|m| {
        if let Some(s) = m.get(id) {
            let _ = s.input.send(event);
        }
    });
}

/// Кладёт текст в буфер сервера и нажимает Shift+Insert.
///
/// Без нажатия текст просто лежал бы в буфере: протокол умеет передать содержимое, но не
/// умеет «вставить». Shift+Insert выбран потому, что работает и в терминалах, где Ctrl+V
/// не делает ничего.
pub fn paste(id: &str, text: String) {
    const SHIFT_L: u32 = 0xffe1;
    const INSERT: u32 = 0xff63;
    with_sessions(|m| {
        let Some(s) = m.get(id) else { return };
        let send = |e: X11Event| {
            let _ = s.input.send(e);
        };
        send(X11Event::CopyText(text));
        send(X11Event::KeyEvent((SHIFT_L, true).into()));
        send(X11Event::KeyEvent((INSERT, true).into()));
        send(X11Event::KeyEvent((INSERT, false).into()));
        send(X11Event::KeyEvent((SHIFT_L, false).into()));
    });
}

/// Переводит выдачу кадров в другое окно и просит перерисовать экран целиком.
///
/// Полный кадр обязателен: сервер RFB присылает только изменения, и новое окно осталось
/// бы с пустым холстом до первого движения на сервере.
pub fn attach(id: &str, ch: Channel<InvokeResponseBody>) -> Result<(), String> {
    // Размер новому окну надо назвать самим. Сервер RFB сообщает его один раз, при
    // подключении, и второй раз не повторит - а без него холста не из чего создать, и
    // все дальнейшие кадры полетели бы в пустоту.
    let size = crate::deskout::active_size(id);
    let found = with_sessions(|m| {
        m.get(id).map(|s| {
            s.out.set(ch);
            if let Some((w, h)) = size {
                let _ = s
                    .out
                    .send(InvokeResponseBody::Raw(packet(kind::RESIZE, 0, 0, w, h, &[])));
            }
            // Именно полный, а не обычный: сервер присылает только изменения, и новое
            // окно осталось бы с пустым холстом до первого движения на сервере.
            s.input.send(X11Event::FullRefresh).ok();
        })
    });
    found.ok_or_else(|| "Этот рабочий стол уже закрыт".to_owned())
}

pub fn close(id: &str) {
    crate::deskout::forget(id);
    with_sessions(|m| {
        if let Some(s) = m.remove(id) {
            s.alive.store(false, Ordering::Relaxed);
        }
    });
}

/// Закрывает все сессии, идущие через указанную SSH-сессию: без этого VNC пережил бы
/// собственный туннель и остался бы висеть с мёртвым каналом.
pub fn close_all() {
    with_sessions(|m| {
        for (id, s) in m.drain() {
            crate::deskout::forget(&id);
            s.alive.store(false, Ordering::Relaxed);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn блокировка_отличается_от_неверного_пароля() {
        // Разбор живого случая на домашнем сервере. TigerVNC после неудачных попыток
        // закрывает доступ и отвечает «Too many security failures» ещё до выбора способа
        // входа - пароль он даже не спрашивает. Раньше эта строка уходила на экран как
        // есть, человек читал её как «пароль опять не тот» и пробовал снова, продлевая
        // блокировку.
        let blocked = VncError::General("Too many security failures".into());
        assert!(is_blacklisted(&blocked));
        assert!(vnc_err(&blocked).contains("временно закрыл доступ"));

        // Обычный отказ по паролю блокировкой не считается: совет там противоположный.
        let wrong = VncError::General("Authentication failed".into());
        assert!(!is_blacklisted(&wrong));
        assert_eq!(vnc_err(&wrong), "Неверный пароль VNC");
    }

    /// Сервер RFB 3.8 с входом по паролю: отдаёт вызов и возвращает ответ клиента на него -
    /// до 16 байт, пока клиент не закрыл соединение.
    async fn сервер_с_паролем(mut s: tokio::io::DuplexStream) -> Vec<u8> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        s.write_all(b"RFB 003.008\n").await.unwrap();
        let mut version = [0u8; 12];
        s.read_exact(&mut version).await.unwrap();
        s.write_all(&[1, 2]).await.unwrap(); // один способ входа: VNC Authentication
        let mut chosen = [0u8; 1];
        s.read_exact(&mut chosen).await.unwrap();
        // Вызов. Клиент без пароля к этому моменту уже ушёл - отвечать ему нечем.
        if s.write_all(&[7u8; 16]).await.is_err() {
            return Vec::new();
        }
        let mut answer = vec![0u8; 16];
        let mut got = 0;
        while got < answer.len() {
            match s.read(&mut answer[got..]).await {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        answer.truncate(got);
        answer
    }

    #[tokio::test]
    async fn без_пароля_серверу_ничего_не_отвечают() {
        // Пустой ответ сервер засчитал бы как неудачный вход и после нескольких закрыл бы
        // доступ - а первое подключение всегда идёт без пароля.
        for пароль in [None, Some(String::new())] {
            let (client, server) = tokio::io::duplex(4096);
            let сервер = tokio::spawn(сервер_с_паролем(server));
            let e = handshake(client, пароль).await.err().expect("без пароля не входим");
            assert!(matches!(e, VncError::NoPassword), "{e:?}");
            let e = OpenError::from(&e);
            assert!(e.needs_password && !e.blacklisted);
            assert!(!e.message.contains("Неверный"), "{}", e.message);
            assert!(сервер.await.unwrap().is_empty(), "ответ на вызов ушёл серверу");
        }
    }

    #[tokio::test]
    async fn с_паролем_отвечают_на_вызов() {
        let (client, server) = tokio::io::duplex(4096);
        let сервер = tokio::spawn(сервер_с_паролем(server));
        let попытка = tokio::spawn(handshake(client, Some("secret".into())));
        let ответ = tokio::time::timeout(std::time::Duration::from_secs(5), сервер)
            .await
            .expect("сервер не дождался ответа")
            .unwrap();
        assert_eq!(ответ.len(), 16, "ответ на вызов - 16 байт");
        // Итога входа сервер так и не прислал - клиент заканчивает обрывом, это ожидаемо.
        let _ = попытка.await;
    }

    #[test]
    fn при_блокировке_форму_пароля_не_предлагают() {
        // Она подсказывала бы сделать ровно то, что блокировку продлевает.
        let e = OpenError::from(&VncError::General("Too many security failures".into()));
        assert!(e.blacklisted);
        assert!(!e.needs_password, "вводить пароль при блокировке бесполезно");
    }
}
