//! Telnet и «сырой» TCP как ещё два типа сессии — рядом с локальным терминалом, SSH и COM.
//!
//! Механика та же, что в `serial.rs`: поток-читатель льёт байты в `TermOut`, обратно пишем
//! прямо в сокет. Разница только в том, что у telnet поверх потока байт живёт согласование
//! опций (RFC 854 и соседи): его надо вычищать из вывода и отвечать на него, иначе на экран
//! посыплется мусор, а сервер будет ждать ответа.
//!
//! «Сырой» TCP — это тот же транспорт без согласования: байты идут как есть. Нужен для
//! консольных серверов (Cisco/Digi слушают порт 2000+ на каждую линию) и для ручного
//! разбирательства с текстовым протоколом.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Сколько ждём установления соединения. Дальше — «хост не отвечает»: у сетевой железки,
/// до которой нет маршрута, системный таймаут тянется десятками секунд.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Каким терминалом представляемся серверу (опция TTYPE).
const TERM_TYPE: &[u8] = b"xterm-256color";

// Команды telnet.
const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

// Опции.
const OPT_BINARY: u8 = 0;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;

/// Подкоманды TTYPE.
const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Telnet,
    Raw,
}

/// Что уходит на сервер при нажатии Enter вне двоичного режима.
///
/// По RFC 854 конец строки в NVT — `CR LF`, это и стоит по умолчанию. Но часть железа
/// (и некоторые BBS-подобные сервисы) ждёт `CR NUL` либо голый `CR`, и на неверном варианте
/// получаются либо двойные переводы строки, либо команда, которая не выполняется.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Eol {
    CrLf,
    CrNul,
    Cr,
}

impl Eol {
    fn from_cfg(s: Option<&str>) -> Eol {
        match s {
            Some("cr-nul") => Eol::CrNul,
            Some("cr") => Eol::Cr,
            _ => Eol::CrLf,
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Eol::CrLf => b"\r\n",
            Eol::CrNul => b"\r\0",
            Eol::Cr => b"\r",
        }
    }
}

pub struct TcpSession {
    sock: Mutex<Option<TcpStream>>,
    alive: Arc<AtomicBool>,
    mode: Mode,
    eol: Eol,
    /// Согласован ли BINARY в нашу сторону: тогда байты идут как есть, без правки перевода строки.
    binary_out: Arc<AtomicBool>,
    /// Согласован ли NAWS: только тогда имеет смысл слать размер окна.
    naws: Arc<AtomicBool>,
}

impl TcpSession {
    pub fn write(&self, data: &str) {
        let payload = match self.mode {
            Mode::Raw => data.as_bytes().to_vec(),
            Mode::Telnet => encode_out(
                data.as_bytes(),
                self.eol,
                self.binary_out.load(Ordering::Relaxed),
            ),
        };
        self.send_raw(&payload);
    }

    fn send_raw(&self, bytes: &[u8]) {
        if let Ok(mut s) = self.sock.lock() {
            if let Some(s) = s.as_mut() {
                let _ = s.write_all(bytes);
                let _ = s.flush();
            }
        }
    }

    /// Размер окна серверу. Молча ничего не делаем, если NAWS не согласован —
    /// незапрошенная подкоманда некоторых серверов сбивает с толку.
    pub fn resize(&self, cols: u16, rows: u16) {
        if self.mode != Mode::Telnet || !self.naws.load(Ordering::Relaxed) {
            return;
        }
        self.send_raw(&naws_payload(cols, rows));
    }

    /// Telnet-команда одной кнопкой: BREAK, Interrupt Process, Are You There.
    /// На сетевом железе это единственный способ прервать зависший `ping` или `traceroute`.
    pub fn send_command(&self, name: &str) -> Result<(), String> {
        if self.mode != Mode::Telnet {
            return Err("Команды telnet доступны только в telnet-сессии".into());
        }
        let code = match name {
            "break" => 243,
            "interrupt" => 244,
            "abort-output" => 245,
            "are-you-there" => 246,
            "erase-char" => 247,
            "erase-line" => 248,
            _ => return Err(format!("Неизвестная команда telnet: {name}")),
        };
        self.send_raw(&[IAC, code]);
        Ok(())
    }

    pub fn close(&self) {
        self.alive.store(false, Ordering::Relaxed);
        if let Ok(mut s) = self.sock.lock() {
            if let Some(s) = s.as_ref() {
                // Разбудит читателя, который висит в блокирующем `read`.
                let _ = s.shutdown(Shutdown::Both);
            }
            s.take();
        }
    }
}

/// Экранирование и перевод строки для потока «от нас к серверу».
///
/// Байт `0xFF` в данных должен уехать удвоенным, иначе сервер примет его за начало команды.
/// Одиночный `CR` (именно его шлёт терминал по Enter) заменяем на выбранную последовательность;
/// уже готовый `CR LF` не трогаем, иначе получится пустая строка.
fn encode_out(data: &[u8], eol: Eol, binary: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        match b {
            IAC => {
                out.push(IAC);
                out.push(IAC);
            }
            b'\r' if !binary => {
                let next = data.get(i + 1).copied();
                if next == Some(b'\n') || next == Some(0) {
                    // Терминал уже прислал готовую пару — отдаём как есть.
                    out.push(b'\r');
                    out.push(next.unwrap());
                    i += 2;
                    continue;
                }
                out.extend_from_slice(eol.bytes());
            }
            _ => out.push(b),
        }
        i += 1;
    }
    out
}

/// `IAC SB NAWS <ширина> <высота> IAC SE`, размеры — big-endian по два байта.
/// Байт `0xFF` внутри подкоманды тоже удваивается — окно шириной 255 иначе рвёт поток.
fn naws_payload(cols: u16, rows: u16) -> Vec<u8> {
    let mut out = vec![IAC, SB, OPT_NAWS];
    for b in [
        (cols >> 8) as u8,
        (cols & 0xff) as u8,
        (rows >> 8) as u8,
        (rows & 0xff) as u8,
    ] {
        out.push(b);
        if b == IAC {
            out.push(IAC);
        }
    }
    out.push(IAC);
    out.push(SE);
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Data,
    Iac,
    Verb(u8),
    SbOpt,
    Sb(u8),
    SbIac(u8),
}

/// Разбор входящего потока telnet: отделяет данные от согласования опций.
///
/// Состояние живёт между чтениями — команда может приехать разрезанной на границе буфера,
/// и без этого её половинки попали бы на экран.
pub struct Negotiator {
    st: St,
    /// Что включено на стороне сервера (по опциям, которые он предлагает).
    him: HashMap<u8, bool>,
    /// Что включено на нашей стороне.
    us: HashMap<u8, bool>,
    sb: Vec<u8>,
    cols: u16,
    rows: u16,
    /// Последний увиденный байт данных — чтобы убрать `NUL` после `CR`.
    prev_cr: bool,
}

/// Что мы согласны делать сами, если сервер попросит (`DO`).
fn we_can_do(opt: u8) -> bool {
    matches!(opt, OPT_TTYPE | OPT_NAWS | OPT_SGA | OPT_BINARY)
}

/// Что мы согласны разрешить серверу, если он предложит (`WILL`).
///
/// `ECHO` со стороны сервера принимаем: это значит «эхо рисую я», и локальное эхо
/// включать не надо — иначе всё вводимое двоится.
fn we_allow(opt: u8) -> bool {
    matches!(opt, OPT_ECHO | OPT_SGA | OPT_BINARY)
}

impl Negotiator {
    pub fn new(cols: u16, rows: u16) -> Negotiator {
        Negotiator {
            st: St::Data,
            him: HashMap::new(),
            us: HashMap::new(),
            sb: Vec::new(),
            cols,
            rows,
            prev_cr: false,
        }
    }

    /// Правда, если сервер согласился присылать нам двоичный поток.
    pub fn binary_in(&self) -> bool {
        *self.him.get(&OPT_BINARY).unwrap_or(&false)
    }

    pub fn binary_out(&self) -> bool {
        *self.us.get(&OPT_BINARY).unwrap_or(&false)
    }

    pub fn naws_on(&self) -> bool {
        *self.us.get(&OPT_NAWS).unwrap_or(&false)
    }

    /// Разбирает кусок входящих байт: `out` — то, что идёт в терминал, `reply` — ответ серверу.
    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>, reply: &mut Vec<u8>) {
        for &b in input {
            match self.st {
                St::Data => {
                    if b == IAC {
                        self.st = St::Iac;
                    } else {
                        self.push_data(b, out);
                    }
                }
                St::Iac => match b {
                    IAC => {
                        // Удвоенный 0xFF — это данные, а не команда.
                        self.push_data(IAC, out);
                        self.st = St::Data;
                    }
                    WILL | WONT | DO | DONT => self.st = St::Verb(b),
                    SB => self.st = St::SbOpt,
                    // NOP, GA, DM и прочие однобайтовые — на экран не выводим.
                    _ => self.st = St::Data,
                },
                St::Verb(v) => {
                    self.negotiate(v, b, reply);
                    self.st = St::Data;
                }
                St::SbOpt => {
                    self.sb.clear();
                    self.st = St::Sb(b);
                }
                St::Sb(opt) => {
                    if b == IAC {
                        self.st = St::SbIac(opt);
                    } else {
                        self.sb.push(b);
                    }
                }
                St::SbIac(opt) => {
                    if b == SE {
                        self.subneg(opt, reply);
                        self.st = St::Data;
                    } else {
                        // `IAC IAC` внутри подкоманды — обычный байт 0xFF.
                        self.sb.push(b);
                        self.st = St::Sb(opt);
                    }
                }
            }
        }
    }

    /// В NVT-режиме `CR NUL` означает «просто возврат каретки». Ноль в терминал не отдаём —
    /// xterm.js его не рисует, но в логах сессии он выглядит как битый файл.
    fn push_data(&mut self, b: u8, out: &mut Vec<u8>) {
        if self.prev_cr && b == 0 && !self.binary_in() {
            self.prev_cr = false;
            return;
        }
        self.prev_cr = b == b'\r';
        out.push(b);
    }

    /// Ответ по RFC 1143: отвечаем только когда состояние опции действительно меняется.
    /// Иначе вежливые серверы уходят в бесконечный обмен `WONT`/`DONT`.
    fn negotiate(&mut self, verb: u8, opt: u8, reply: &mut Vec<u8>) {
        match verb {
            WILL => {
                let want = we_allow(opt);
                let cur = *self.him.get(&opt).unwrap_or(&false);
                if want && !cur {
                    self.him.insert(opt, true);
                    reply.extend_from_slice(&[IAC, DO, opt]);
                } else if !want && self.him.get(&opt) != Some(&false) {
                    self.him.insert(opt, false);
                    reply.extend_from_slice(&[IAC, DONT, opt]);
                }
            }
            WONT => {
                if self.him.get(&opt) != Some(&false) {
                    self.him.insert(opt, false);
                    reply.extend_from_slice(&[IAC, DONT, opt]);
                }
            }
            DO => {
                let can = we_can_do(opt);
                let cur = *self.us.get(&opt).unwrap_or(&false);
                if can && !cur {
                    self.us.insert(opt, true);
                    reply.extend_from_slice(&[IAC, WILL, opt]);
                    // Размер окна нужен серверу сразу: без него `less` и `top` рисуют по 80x24.
                    if opt == OPT_NAWS {
                        reply.extend_from_slice(&naws_payload(self.cols, self.rows));
                    }
                } else if !can && self.us.get(&opt) != Some(&false) {
                    self.us.insert(opt, false);
                    reply.extend_from_slice(&[IAC, WONT, opt]);
                }
            }
            DONT => {
                if self.us.get(&opt) != Some(&false) {
                    self.us.insert(opt, false);
                    reply.extend_from_slice(&[IAC, WONT, opt]);
                }
            }
            _ => {}
        }
    }

    fn subneg(&mut self, opt: u8, reply: &mut Vec<u8>) {
        if opt == OPT_TTYPE && self.sb.first() == Some(&TTYPE_SEND) {
            reply.extend_from_slice(&[IAC, SB, OPT_TTYPE, TTYPE_IS]);
            reply.extend_from_slice(TERM_TYPE);
            reply.extend_from_slice(&[IAC, SE]);
        }
        self.sb.clear();
    }
}

/// Строка под шапкой сессии: куда подключились и в каком режиме.
pub fn describe(mode: Mode, host: &str, port: u16) -> String {
    match mode {
        Mode::Telnet => format!("telnet {host}:{port}"),
        Mode::Raw => format!("TCP {host}:{port} · без согласования"),
    }
}

/// Понятный текст вместо системного кода: «отказано в соединении» и «нет маршрута»
/// лечатся по-разному, и первое, что нужно понять, — какое из двух.
fn connect_error(host: &str, port: u16, e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        ConnectionRefused => {
            format!("{host}:{port} отказал в соединении — на этом порту никто не слушает")
        }
        TimedOut => format!("{host}:{port} не отвечает — истекло время ожидания"),
        _ => format!("Не удалось подключиться к {host}:{port}: {e}"),
    }
}

/// Открывает TCP-сессию (telnet или сырую) и запускает поток-читатель.
pub fn open_tcp(
    app: AppHandle,
    id: String,
    mode: Mode,
    host: &str,
    port: u16,
    eol_cfg: Option<&str>,
    cols: u16,
    rows: u16,
) -> Result<TcpSession, String> {
    if host.trim().is_empty() {
        return Err("Не указан адрес хоста".into());
    }
    // Резолвим сами: `connect_timeout` умеет только готовый адрес, а без таймаута
    // недоступная железка держит окно «подключаюсь» до системного предела.
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("Не удалось разобрать адрес {host}: {e}"))?
        .next()
        .ok_or_else(|| format!("Имя {host} не разрешается в адрес"))?;

    let sock = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|e| connect_error(host, port, &e))?;
    // Без этого каждое нажатие клавиши ждёт подтверждения предыдущего — на медленном
    // канале ввод начинает заметно отставать.
    let _ = sock.set_nodelay(true);

    let reader_sock = sock
        .try_clone()
        .map_err(|e| format!("Не удалось открыть сокет на чтение: {e}"))?;
    let reply_sock = sock
        .try_clone()
        .map_err(|e| format!("Не удалось открыть сокет для ответов: {e}"))?;

    let alive = Arc::new(AtomicBool::new(true));
    let binary_out = Arc::new(AtomicBool::new(false));
    let naws = Arc::new(AtomicBool::new(false));

    spawn_reader(
        app,
        id,
        mode,
        reader_sock,
        reply_sock,
        alive.clone(),
        binary_out.clone(),
        naws.clone(),
        cols,
        rows,
    );

    Ok(TcpSession {
        sock: Mutex::new(Some(sock)),
        alive,
        mode,
        eol: Eol::from_cfg(eol_cfg),
        binary_out,
        naws,
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_reader(
    app: AppHandle,
    id: String,
    mode: Mode,
    mut reader: TcpStream,
    mut reply_sock: TcpStream,
    alive: Arc<AtomicBool>,
    binary_out: Arc<AtomicBool>,
    naws: Arc<AtomicBool>,
    cols: u16,
    rows: u16,
) {
    let out = crate::term_out::TermOut::spawn(app.clone(), id.clone());
    std::thread::spawn(move || {
        let mut neg = Negotiator::new(cols, rows);
        let mut buf = [0u8; 8192];
        let mut data = Vec::with_capacity(8192);
        let mut reply = Vec::new();
        let mut error: Option<String> = None;

        while alive.load(Ordering::Relaxed) {
            match reader.read(&mut buf) {
                // Ноль байт от TCP — это закрытие с той стороны, а не тишина.
                Ok(0) => {
                    if alive.load(Ordering::Relaxed) {
                        error = Some("Соединение закрыто удалённой стороной".into());
                    }
                    break;
                }
                Ok(n) => match mode {
                    Mode::Raw => out.push(&buf[..n]),
                    Mode::Telnet => {
                        data.clear();
                        reply.clear();
                        neg.feed(&buf[..n], &mut data, &mut reply);
                        if !reply.is_empty() && reply_sock.write_all(&reply).is_err() {
                            break;
                        }
                        binary_out.store(neg.binary_out(), Ordering::Relaxed);
                        naws.store(neg.naws_on(), Ordering::Relaxed);
                        if !data.is_empty() {
                            out.push(&data);
                        }
                    }
                },
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => {
                    if alive.load(Ordering::Relaxed) {
                        error = Some(format!("Соединение потеряно: {e}"));
                    }
                    break;
                }
            }
        }

        out.close();
        out.join_blocking();
        let _ = app.emit(
            "session-exit",
            json!({ "id": id, "code": null, "signal": null, "error": error }),
        );
    });
}

/// Адрес и порт из профиля либо из разовых параметров окна подключения.
pub fn endpoint(p: &Value, default_port: u16) -> (String, u16) {
    let host = p
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let port = p
        .get("port")
        .and_then(|v| v.as_u64())
        .filter(|n| *n > 0 && *n <= 65535)
        .unwrap_or(default_port as u64) as u16;
    (host, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(neg: &mut Negotiator, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut out = Vec::new();
        let mut reply = Vec::new();
        neg.feed(input, &mut out, &mut reply);
        (out, reply)
    }

    #[test]
    fn plain_bytes_pass_through() {
        let mut n = Negotiator::new(80, 24);
        let (out, reply) = feed(&mut n, b"login: ");
        assert_eq!(out, b"login: ");
        assert!(reply.is_empty());
    }

    #[test]
    fn doubled_iac_is_data() {
        let mut n = Negotiator::new(80, 24);
        let (out, _) = feed(&mut n, &[b'a', IAC, IAC, b'b']);
        assert_eq!(out, vec![b'a', 0xff, b'b']);
    }

    #[test]
    fn we_answer_the_options_we_support() {
        let mut n = Negotiator::new(100, 40);
        // Сервер просит нас слать тип терминала и размер окна и предлагает своё эхо.
        let (out, reply) = feed(
            &mut n,
            &[IAC, DO, OPT_TTYPE, IAC, DO, OPT_NAWS, IAC, WILL, OPT_ECHO],
        );
        assert!(out.is_empty(), "согласование не должно попадать на экран");

        let mut want = vec![IAC, WILL, OPT_TTYPE, IAC, WILL, OPT_NAWS];
        want.extend_from_slice(&naws_payload(100, 40));
        want.extend_from_slice(&[IAC, DO, OPT_ECHO]);
        assert_eq!(reply, want);
        assert!(n.naws_on());
    }

    #[test]
    fn unsupported_options_are_refused_once() {
        let mut n = Negotiator::new(80, 24);
        // 32 — TSPEED, мы её не умеем.
        let (_, first) = feed(&mut n, &[IAC, DO, 32]);
        assert_eq!(first, vec![IAC, WONT, 32]);
        // Повторный запрос состояние не меняет — молчим, иначе получится бесконечный обмен.
        let (_, second) = feed(&mut n, &[IAC, DO, 32]);
        assert!(second.is_empty());
    }

    #[test]
    fn terminal_type_is_reported_on_request() {
        let mut n = Negotiator::new(80, 24);
        feed(&mut n, &[IAC, DO, OPT_TTYPE]);
        let (out, reply) = feed(&mut n, &[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
        assert!(out.is_empty());
        let mut want = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        want.extend_from_slice(TERM_TYPE);
        want.extend_from_slice(&[IAC, SE]);
        assert_eq!(reply, want);
    }

    #[test]
    fn command_split_across_reads_is_still_parsed() {
        let mut n = Negotiator::new(80, 24);
        // Команда приезжает разрезанной ровно посередине — так и бывает на реальном сокете.
        let (out1, reply1) = feed(&mut n, &[b'x', IAC]);
        assert_eq!(out1, b"x");
        assert!(reply1.is_empty());
        let (out2, reply2) = feed(&mut n, &[DO, OPT_SGA]);
        assert!(out2.is_empty());
        assert_eq!(reply2, vec![IAC, WILL, OPT_SGA]);
    }

    #[test]
    fn cr_nul_loses_the_zero() {
        let mut n = Negotiator::new(80, 24);
        let (out, _) = feed(&mut n, b"a\r\0b");
        assert_eq!(out, b"a\rb");
    }

    #[test]
    fn outgoing_iac_is_doubled() {
        let got = encode_out(&[b'a', 0xff, b'b'], Eol::CrLf, false);
        assert_eq!(got, vec![b'a', 0xff, 0xff, b'b']);
    }

    #[test]
    fn enter_becomes_the_configured_line_ending() {
        assert_eq!(encode_out(b"ip\r", Eol::CrLf, false), b"ip\r\n".to_vec());
        assert_eq!(encode_out(b"ip\r", Eol::CrNul, false), b"ip\r\0".to_vec());
        assert_eq!(encode_out(b"ip\r", Eol::Cr, false), b"ip\r".to_vec());
        // Готовую пару не трогаем, иначе выйдет лишняя пустая строка.
        assert_eq!(encode_out(b"ip\r\n", Eol::CrLf, false), b"ip\r\n".to_vec());
        // В двоичном режиме перевод строки не наша забота.
        assert_eq!(encode_out(b"ip\r", Eol::CrLf, true), b"ip\r".to_vec());
    }

    #[test]
    fn naws_escapes_the_iac_byte() {
        // Ширина 255 даёт в младшем байте 0xFF — его надо удвоить, иначе поток порвётся.
        let p = naws_payload(255, 24);
        assert_eq!(p, vec![IAC, SB, OPT_NAWS, 0, 255, 255, 0, 24, IAC, SE]);
    }

    #[test]
    fn endpoint_falls_back_to_the_default_port() {
        assert_eq!(endpoint(&json!({ "host": " sw1 " }), 23), ("sw1".into(), 23));
        assert_eq!(
            endpoint(&json!({ "host": "sw1", "port": 2001 }), 23),
            ("sw1".into(), 2001)
        );
        // Мусорный порт не должен уехать в соединение.
        assert_eq!(
            endpoint(&json!({ "host": "sw1", "port": 0 }), 23),
            ("sw1".into(), 23)
        );
    }

    #[test]
    fn eol_parsing_defaults_to_crlf() {
        assert_eq!(Eol::from_cfg(Some("cr-nul")), Eol::CrNul);
        assert_eq!(Eol::from_cfg(Some("cr")), Eol::Cr);
        assert_eq!(Eol::from_cfg(None), Eol::CrLf);
        assert_eq!(Eol::from_cfg(Some("мусор")), Eol::CrLf);
    }

    /// Разговор с настоящим telnet-сервером через настоящий сокет.
    ///
    /// Поднимается `node scripts/telnetd.js 2323` — он ведёт себя как консоль сетевой
    /// железки: сам навязывает `WILL ECHO`/`WILL SGA`, требует `DO TTYPE`/`DO NAWS`
    /// и заведомо неподдерживаемый `DO TSPEED`. Если сервер не запущен, тест молча
    /// пропускается — как с виртуальной парой COM-портов.
    #[test]
    fn talks_to_a_real_telnet_server() {
        use std::io::Write;
        use std::net::TcpStream;

        let addr = match "127.0.0.1:2323".parse() {
            Ok(a) => a,
            Err(_) => return,
        };
        let mut sock = match TcpStream::connect_timeout(&addr, Duration::from_millis(700)) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("пропуск: на 127.0.0.1:2323 никто не слушает (node scripts/telnetd.js)");
                return;
            }
        };
        sock.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        let mut reply_sock = sock.try_clone().unwrap();

        let mut neg = Negotiator::new(120, 40);
        let mut screen: Vec<u8> = Vec::new();
        // Тот же цикл, что и в потоке-читателе: данные — на экран, ответы — в сокет.
        fn pump(
            sock: &mut TcpStream,
            back: &mut TcpStream,
            neg: &mut Negotiator,
            screen: &mut Vec<u8>,
            rounds: usize,
        ) {
            let mut buf = [0u8; 4096];
            for _ in 0..rounds {
                match sock.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut data = Vec::new();
                        let mut reply = Vec::new();
                        neg.feed(&buf[..n], &mut data, &mut reply);
                        if !reply.is_empty() {
                            back.write_all(&reply).unwrap();
                        }
                        screen.extend_from_slice(&data);
                    }
                    Err(_) => {}
                }
            }
        }

        pump(&mut sock, &mut reply_sock, &mut neg, &mut screen, 8);

        // Мы согласились сообщать размер окна и получили приглашение сервера.
        assert!(neg.naws_on(), "NAWS должен быть согласован");
        let text = String::from_utf8_lossy(&screen);
        assert!(
            text.contains("login:"),
            "приглашение сервера не дошло, на экране: {text:?}"
        );
        // Согласование не должно попадать в вывод.
        assert!(!screen.contains(&IAC), "байты согласования протекли на экран");

        // Отправляем строку и ждём эхо (сервер объявил WILL ECHO).
        reply_sock
            .write_all(&encode_out(b"admin\r", Eol::CrLf, neg.binary_out()))
            .unwrap();
        screen.clear();
        pump(&mut sock, &mut reply_sock, &mut neg, &mut screen, 8);
        let echo = String::from_utf8_lossy(&screen);
        assert!(echo.contains("admin"), "эхо не пришло, на экране: {echo:?}");
    }

    /// «Сырой» TCP к тому же серверу: обработки нет, значит байты согласования
    /// должны прийти как есть. Это и отличает режим от telnet.
    #[test]
    fn raw_mode_passes_negotiation_bytes_through() {
        use std::net::TcpStream;

        let addr = match "127.0.0.1:2323".parse() {
            Ok(a) => a,
            Err(_) => return,
        };
        let mut sock = match TcpStream::connect_timeout(&addr, Duration::from_millis(700)) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("пропуск: на 127.0.0.1:2323 никто не слушает (node scripts/telnetd.js)");
                return;
            }
        };
        sock.set_read_timeout(Some(Duration::from_millis(400))).unwrap();

        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        for _ in 0..4 {
            match sock.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(_) => {}
            }
        }
        assert!(
            got.contains(&IAC),
            "в сыром режиме согласование должно доходить нетронутым, пришло: {got:?}"
        );
        assert!(
            got.windows(3).any(|w| w == [IAC, WILL, OPT_ECHO]),
            "не нашли предложение сервера включить эхо"
        );
    }

    #[test]
    fn describe_names_the_mode() {
        assert_eq!(describe(Mode::Telnet, "10.0.0.1", 23), "telnet 10.0.0.1:23");
        assert_eq!(
            describe(Mode::Raw, "10.0.0.1", 2001),
            "TCP 10.0.0.1:2001 · без согласования"
        );
    }
}
