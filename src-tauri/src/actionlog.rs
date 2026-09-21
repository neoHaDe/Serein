//! Журнал действий: кто, когда, на каком сервере и что сделал.
//!
//! Спрашивают его первым после шифрования: служба безопасности хочет знать, что делали на
//! серверах из этого приложения. Поэтому пишется всё, что меняет сервер или ходит на него, -
//! подключения, файлы, службы, контейнеры, базы, Fleet и задачи, - и строки, набранные в
//! терминале.
//!
//! Записи лежат построчно в помесячных файлах `action-log/actions-ГГГГ-ММ.jsonl` в папке
//! настроек. Каждая несёт номер и хеш предыдущей: удалить или подменить строку незаметно
//! нельзя - проверка найдёт место разрыва. Это не защита от того, кто перепишет журнал
//! целиком с пересчётом хешей, - для этого журнал уходит ещё и в syslog компании, куда у
//! пользователя машины доступа нет.
//!
//! Чего журнал не знает. Строка терминала - это набранное с клавиатуры: автодополнение и
//! история стрелками подставляют текст на стороне сервера, и такая строка помечается. Строка,
//! набранная после запроса пароля, не пишется вовсе.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DIR: &str = "action-log";
/// Текст команды или запроса в записи. Длиннее - обрезается с пометкой.
const MAX_TEXT: usize = 8 * 1024;
/// Набранная строка терминала: вставленный мегабайт не должен уйти в журнал целиком.
const MAX_LINE: usize = 8 * 1024;

static ENABLED: AtomicBool = AtomicBool::new(true);
static WRITER: Mutex<Option<Chain>> = Mutex::new(None);
static SESSIONS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
static LINES: Mutex<Option<HashMap<String, LineBuf>>> = Mutex::new(None);
static SYSLOG: Mutex<Option<(SyslogCfg, Sender<String>)>> = Mutex::new(None);
static SYSLOG_SENT: AtomicU64 = AtomicU64::new(0);
static SYSLOG_FAILED: AtomicU64 = AtomicU64::new(0);
static WRITE_FAILED: AtomicU64 = AtomicU64::new(0);
static LAST_WRITE_ERROR: Mutex<Option<String>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn dir() -> PathBuf {
    crate::store::config_dir().join(DIR)
}

/// Где лежит журнал - для окна просмотра.
pub fn dir_path() -> String {
    dir().to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------------------
// Время и кто

fn now_parts() -> (i64, u32) {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() as i64, d.subsec_millis())
}

/// Дата по дням от эпохи - алгоритм Хиннанта, как в `term_out::stamp_utc`.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// `2026-09-15T10:20:30.123Z` и имя месячного файла.
fn stamp(secs: i64, millis: u32) -> (String, String) {
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (rem / 3600, rem % 3600 / 60, rem % 60);
    (
        format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{millis:03}Z"),
        format!("actions-{y:04}-{m:02}.jsonl"),
    )
}

fn actor() -> Value {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let machine = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_owned()))
        .unwrap_or_default();
    json!({ "user": user, "machine": machine })
}

/// Сервер из списка: имя, адрес, порт, пользователь. Секретов здесь нет и быть не должно.
fn server_info(id: &str) -> Value {
    crate::store::servers_list()
        .into_iter()
        .find(|s| s.get("id").and_then(Value::as_str) == Some(id))
        .map(|s| {
            json!({
                "id": id,
                "name": s.get("name").cloned().unwrap_or(Value::Null),
                "host": s.get("host").cloned().unwrap_or(Value::Null),
                "port": s.get("port").cloned().unwrap_or(Value::Null),
                "user": s.get("username").cloned().unwrap_or(Value::Null),
            })
        })
        .unwrap_or_else(|| json!({ "id": id }))
}

/// Текст для записи: не длиннее предела, обрезка видна.
pub fn text(s: &str) -> Value {
    if s.len() <= MAX_TEXT {
        return json!(s);
    }
    let mut cut = MAX_TEXT;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    json!(format!("{}… (обрезано, всего {} Б)", &s[..cut], s.len()))
}

// ---------------------------------------------------------------------------------------
// Цепочка

struct Chain {
    seq: u64,
    last: String,
}

/// Хеш записи: предыдущий хеш и сама запись без поля `hash`.
fn digest(prev: &str, body: &Value) -> String {
    let mut h = Sha256::new();
    h.update(prev.as_bytes());
    h.update(b"\n");
    h.update(serde_json::to_string(body).unwrap_or_default().as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn log_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("actions-") && n.ends_with(".jsonl"))
        })
        .collect();
    // Имена вида `actions-ГГГГ-ММ` сортируются как даты.
    files.sort();
    files
}

/// Последняя строка файла - читаем с конца, файл за месяц может быть большим.
fn last_line(path: &PathBuf) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let take = len.min(256 * 1024);
    f.seek(SeekFrom::Start(len - take)).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;
    buf.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_owned)
}

fn resume() -> Chain {
    let last = log_files()
        .last()
        .and_then(last_line)
        .and_then(|l| serde_json::from_str::<Value>(&l).ok());
    match last {
        Some(v) => Chain {
            seq: v["seq"].as_u64().unwrap_or(0),
            last: v["hash"].as_str().unwrap_or("").to_owned(),
        },
        None => Chain {
            seq: 0,
            last: String::new(),
        },
    }
}

/// Журнал перестал писаться - это само по себе событие.
///
/// Снаружи «журнал не пишется» выглядит ровно как «ничего не происходило», поэтому неудачи
/// считаются, последняя причина держится для окна, а при настроенном syslog уходит и туда:
/// там как раз тот, кому положено об этом узнать.
fn note_write_failure(t: &str, reason: &str) {
    WRITE_FAILED.fetch_add(1, Ordering::Relaxed);
    *lock(&LAST_WRITE_ERROR) = Some(reason.to_owned());
    eprintln!("журнал действий: запись не легла: {reason}");
    if let Some((_, tx)) = lock(&SYSLOG).as_ref() {
        let body = json!({
            "t": t,
            "actor": actor(),
            "action": "journal.write.failed",
            "ok": false,
            "error": reason,
        });
        let _ = tx.send(syslog_message(&body));
    }
}

fn write_entry(server: Option<&str>, session: Option<&str>, action: &str, detail: Value, result: Result<(), String>) {
    let (secs, millis) = now_parts();
    let (t, file) = stamp(secs, millis);
    let mut guard = lock(&WRITER);
    let chain = guard.get_or_insert_with(resume);
    let seq = chain.seq + 1;
    let mut body = json!({
        "seq": seq,
        "t": t,
        "actor": actor(),
        "server": server.map(server_info),
        "session": session,
        "action": action,
        "detail": detail,
        "ok": result.is_ok(),
        "error": result.err(),
        "prev": chain.last,
    });
    let hash = digest(&chain.last, &body);
    body["hash"] = json!(hash);
    let line = serde_json::to_string(&body).unwrap_or_default();
    let written = fs::create_dir_all(dir()).and_then(|_| {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir().join(&file))?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.flush()
    });
    match written {
        Ok(()) => {
            chain.seq = seq;
            chain.last = hash;
        }
        // Не записали - цепочку не двигаем: следующая запись продолжит от последней на диске.
        Err(e) => note_write_failure(&t, &e.to_string()),
    }
    drop(guard);
    if let Some((_, tx)) = lock(&SYSLOG).as_ref() {
        let _ = tx.send(syslog_message(&body));
    }
}

/// Записать действие, если журнал включён.
pub fn record(server: Option<&str>, session: Option<&str>, action: &str, detail: Value, result: Result<(), String>) {
    if ENABLED.load(Ordering::Relaxed) {
        write_entry(server, session, action, detail, result);
    }
}

/// Действие в сессии: сервер находится по сессии. Сессии без сервера (локальный терминал)
/// не пишутся - на чужие серверы из них не ходят.
pub fn record_session<T, E: std::fmt::Display>(session: &str, action: &str, detail: Value, result: &Result<T, E>) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Some(server) = server_of(session) else { return };
    write_entry(Some(&server), Some(session), action, detail, outcome(result));
}

pub fn outcome<T, E: std::fmt::Display>(r: &Result<T, E>) -> Result<(), String> {
    match r {
        Ok(_) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn bind(session: &str, server: &str) {
    lock(&SESSIONS)
        .get_or_insert_with(HashMap::new)
        .insert(session.to_owned(), server.to_owned());
}

pub fn unbind(session: &str) -> Option<String> {
    lock(&LINES).get_or_insert_with(HashMap::new).remove(session);
    lock(&SESSIONS).get_or_insert_with(HashMap::new).remove(session)
}

pub fn server_of(session: &str) -> Option<String> {
    lock(&SESSIONS).as_ref().and_then(|m| m.get(session).cloned())
}

// ---------------------------------------------------------------------------------------
// Настройки

#[derive(Clone, PartialEq, Debug)]
struct SyslogCfg {
    host: String,
    port: u16,
    tcp: bool,
}

fn syslog_cfg(settings: &Value) -> Option<SyslogCfg> {
    let s = settings.get("actionLogSyslog")?;
    if s.get("enabled").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let host = s.get("host").and_then(Value::as_str)?.trim().to_owned();
    let port = u16::try_from(s.get("port").and_then(Value::as_u64).unwrap_or(514)).ok()?;
    if host.is_empty() || port == 0 {
        return None;
    }
    let tcp = s.get("protocol").and_then(Value::as_str) == Some("tcp");
    Some(SyslogCfg { host, port, tcp })
}

/// Настройки при запуске: без записей о включении и выключении - состояние не менялось.
pub fn init(settings: &Value) {
    apply(settings, false);
}

/// Применяет изменённые настройки. Выключение пишется последней записью, включение -
/// первой: пропуск в журнале должен быть объяснён в самом журнале.
pub fn configure(settings: &Value) {
    apply(settings, true);
}

fn apply(settings: &Value, announce: bool) {
    let on = settings.get("actionLog").and_then(Value::as_bool).unwrap_or(true);
    let was = ENABLED.load(Ordering::Relaxed);
    if announce && was && !on {
        write_entry(None, None, "journal.disabled", json!({}), Ok(()));
    }
    ENABLED.store(on, Ordering::Relaxed);

    let want = if on { syslog_cfg(settings) } else { None };
    {
        let mut cur = lock(&SYSLOG);
        if cur.as_ref().map(|(c, _)| c) != want.as_ref() {
            // Старый поток уходит сам, как только закрывается его канал.
            *cur = want.map(|cfg| (cfg.clone(), spawn_syslog(cfg)));
        }
    }
    if announce && !was && on {
        write_entry(None, None, "journal.enabled", json!({}), Ok(()));
    }
}

// ---------------------------------------------------------------------------------------
// syslog

/// RFC 5424: local0.info, приложение `Serein`, запись целиком в JSON.
fn syslog_message(entry: &Value) -> String {
    let t = entry["t"].as_str().unwrap_or("-");
    let machine = entry["actor"]["machine"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("-");
    let action = entry["action"].as_str().unwrap_or("-");
    let host: String = machine.chars().filter(|c| c.is_ascii_graphic()).take(255).collect();
    let msgid: String = action.chars().filter(|c| c.is_ascii_graphic()).take(32).collect();
    format!(
        "<134>1 {t} {host} Serein - {msgid} - {}",
        serde_json::to_string(entry).unwrap_or_default()
    )
}

fn spawn_syslog(cfg: SyslogCfg) -> Sender<String> {
    let (tx, rx) = channel::<String>();
    std::thread::Builder::new()
        .name("action-log-syslog".into())
        .spawn(move || {
            let addr = format!("{}:{}", cfg.host, cfg.port);
            let udp = if cfg.tcp {
                None
            } else {
                std::net::UdpSocket::bind("0.0.0.0:0").ok()
            };
            let mut tcp: Option<std::net::TcpStream> = None;
            for msg in rx {
                let sent = if cfg.tcp {
                    // RFC 6587: длина перед сообщением - переводы строк внутри JSON не рвут поток.
                    let framed = format!("{} {msg}", msg.len());
                    let mut ok = tcp.as_mut().is_some_and(|s| s.write_all(framed.as_bytes()).is_ok());
                    if !ok {
                        tcp = connect_tcp(&addr);
                        ok = tcp.as_mut().is_some_and(|s| s.write_all(framed.as_bytes()).is_ok());
                    }
                    ok
                } else {
                    udp.as_ref().is_some_and(|s| s.send_to(msg.as_bytes(), &addr).is_ok())
                };
                if sent {
                    SYSLOG_SENT.fetch_add(1, Ordering::Relaxed);
                } else {
                    SYSLOG_FAILED.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
        .expect("поток отправки журнала запускается");
    tx
}

fn connect_tcp(addr: &str) -> Option<std::net::TcpStream> {
    use std::net::ToSocketAddrs as _;
    let sa = addr.to_socket_addrs().ok()?.next()?;
    let s = std::net::TcpStream::connect_timeout(&sa, Duration::from_secs(3)).ok()?;
    let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
    Some(s)
}

// ---------------------------------------------------------------------------------------
// Терминал

#[derive(Default, Debug)]
enum Esc {
    #[default]
    None,
    Start,
    Csi(String),
    Ss3,
}

/// Набираемая строка одной сессии.
#[derive(Default, Debug)]
pub struct LineBuf {
    text: String,
    /// Стрелки, Tab, Ctrl-сочетания: строку дописал или заменил сервер, и в журнале только
    /// то, что набрано с клавиатуры.
    edited: bool,
    esc: Esc,
}

impl LineBuf {
    /// Готовые строки - по каждому Enter.
    pub fn feed(&mut self, data: &str) -> Vec<(String, bool)> {
        let mut done = Vec::new();
        for c in data.chars() {
            match std::mem::take(&mut self.esc) {
                Esc::Start => {
                    self.esc = match c {
                        '[' => Esc::Csi(String::new()),
                        'O' => Esc::Ss3,
                        _ => {
                            self.edited = true;
                            Esc::None
                        }
                    };
                    continue;
                }
                Esc::Csi(mut params) => {
                    if ('\x40'..='\x7e').contains(&c) {
                        // Вставка из буфера (`200~` … `201~`) - это набранный текст, всё
                        // остальное - перемещение курсора и история.
                        if !(c == '~' && (params == "200" || params == "201")) {
                            self.edited = true;
                        }
                    } else {
                        params.push(c);
                        self.esc = Esc::Csi(params);
                    }
                    continue;
                }
                Esc::Ss3 => {
                    self.edited = true;
                    continue;
                }
                Esc::None => {}
            }
            match c {
                '\x1b' => self.esc = Esc::Start,
                '\r' | '\n' => {
                    if !self.text.trim().is_empty() || self.edited {
                        done.push((std::mem::take(&mut self.text), self.edited));
                    }
                    self.text.clear();
                    self.edited = false;
                }
                '\x7f' | '\x08' => {
                    self.text.pop();
                }
                '\x03' | '\x15' => {
                    self.text.clear();
                    self.edited = false;
                }
                '\x17' => {
                    let keep = self.text.trim_end().rfind(' ').map_or(0, |i| i + 1);
                    self.text.truncate(keep);
                }
                c if (c as u32) < 0x20 => self.edited = true,
                c => {
                    if self.text.len() < MAX_LINE {
                        self.text.push(c);
                    }
                }
            }
        }
        done
    }
}

/// Похоже ли последнее, что вывел сервер, на запрос пароля.
pub fn asks_secret(tail: &str) -> bool {
    let plain = strip_ansi(tail);
    let last = plain
        .trim_end_matches(['\r', '\n', ' '])
        .rsplit(['\n', '\r'])
        .next()
        .unwrap_or("");
    let l = last.to_lowercase();
    let secret = [
        "password",
        "passphrase",
        "passcode",
        "пароль",
        "verification code",
        "one-time",
        "otp",
        "pin:",
    ]
    .iter()
    .any(|w| l.contains(w));
    secret && (l.trim_end().ends_with(':') || l.trim_end().ends_with('?') || l.trim_end().ends_with('>'))
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&n) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Ввод в терминал SSH-сессии: строки по Enter.
pub fn terminal_input(session: &str, data: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Some(server) = server_of(session) else { return };
    let lines = {
        let mut map = lock(&LINES);
        map.get_or_insert_with(HashMap::new)
            .entry(session.to_owned())
            .or_default()
            .feed(data)
    };
    for (line, edited) in lines {
        let detail = if asks_secret(&crate::term_out::replay(session)) {
            json!({ "hidden": "набрано после запроса пароля - не записано" })
        } else {
            json!({ "line": line, "edited": edited })
        };
        write_entry(Some(&server), Some(session), "terminal.line", detail, Ok(()));
    }
}

// ---------------------------------------------------------------------------------------
// Просмотр, проверка, выгрузка

/// Последние записи, новые первыми.
pub fn list(limit: usize) -> Vec<Value> {
    let mut out = Vec::new();
    for file in log_files().iter().rev() {
        let Ok(f) = fs::File::open(file) else { continue };
        let mut lines: Vec<Value> = BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect();
        lines.reverse();
        for v in lines {
            out.push(v);
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

/// Проверка цепочки по всем файлам: номера подряд, хеши сходятся.
pub fn verify() -> Value {
    let mut prev = String::new();
    let mut seq = 0u64;
    let files = log_files();
    for file in &files {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned();
        let Ok(f) = fs::File::open(file) else {
            return json!({ "ok": false, "count": seq, "file": name, "line": 0, "reason": "файл не открывается" });
        };
        for (i, line) in BufReader::new(f).lines().enumerate() {
            let fail =
                |reason: &str| json!({ "ok": false, "count": seq, "file": name, "line": i + 1, "reason": reason });
            let Ok(line) = line else {
                return fail("строка не читается");
            };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(mut v) = serde_json::from_str::<Value>(&line) else {
                return fail("строка повреждена");
            };
            let hash = v.as_object_mut().and_then(|o| o.remove("hash"));
            if v["seq"].as_u64() != Some(seq + 1) {
                return fail("номер записи не по порядку - записи удалены или вставлены");
            }
            if v["prev"].as_str() != Some(prev.as_str()) {
                return fail("ссылка на предыдущую запись не сходится - записи до неё удалены или изменены");
            }
            let expect = digest(&prev, &v);
            if hash.as_ref().and_then(Value::as_str) != Some(expect.as_str()) {
                return fail("хеш не сходится - запись изменена");
            }
            prev = expect;
            seq += 1;
        }
    }
    json!({ "ok": true, "count": seq, "files": files.len() })
}

/// Весь журнал одним файлом JSONL.
pub fn export(dest: &str) -> Result<usize, String> {
    let mut out = fs::File::create(dest).map_err(|e| format!("не создать {dest}: {e}"))?;
    let mut n = 0;
    for file in log_files() {
        let f = fs::File::open(&file).map_err(|e| e.to_string())?;
        for line in BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.trim().is_empty())
        {
            out.write_all(line.as_bytes())
                .and_then(|_| out.write_all(b"\n"))
                .map_err(|e| e.to_string())?;
            n += 1;
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    Ok(n)
}

pub fn status() -> Value {
    json!({
        "enabled": ENABLED.load(Ordering::Relaxed),
        "dir": dir_path(),
        "syslog": lock(&SYSLOG).as_ref().map(|(c, _)| json!({ "host": c.host, "port": c.port, "tcp": c.tcp })),
        "syslogSent": SYSLOG_SENT.load(Ordering::Relaxed),
        "syslogFailed": SYSLOG_FAILED.load(Ordering::Relaxed),
        "writeFailed": WRITE_FAILED.load(Ordering::Relaxed),
        "lastWriteError": lock(&LAST_WRITE_ERROR).clone(),
        // Замок, доставшийся отравленным, означает панику в критической секции: состояние
        // могло остаться на половине правки, и это стоит видеть рядом с журналом.
        "locksPoisoned": crate::sync::poisoned_count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn сбой_записи_журнала_виден_в_статусе() {
        // Раньше о неудаче знала только консоль, которой у приложения нет: журнал,
        // переставший писаться, выглядел как журнал, в котором ничего не происходило.
        let было = status()["writeFailed"].as_u64().unwrap_or(0);
        note_write_failure("2026-09-21T00:00:00.000Z", "каталог только для чтения");
        let s = status();
        assert_eq!(s["writeFailed"].as_u64(), Some(было + 1));
        assert_eq!(s["lastWriteError"].as_str(), Some("каталог только для чтения"));
    }

    #[test]
    fn набранная_строка_собирается_по_enter_с_правкой_и_отменой() {
        let mut b = LineBuf::default();
        assert_eq!(b.feed("ls -la"), vec![]);
        assert_eq!(
            b.feed("x\x7f\r"),
            vec![("ls -la".to_owned(), false)],
            "Backspace убирает символ"
        );
        assert_eq!(b.feed("rm -rf /tmp/x\x03"), vec![], "Ctrl+C - строка не выполнена");
        assert_eq!(b.feed("\r"), vec![], "пустой Enter не пишется");
        assert_eq!(
            b.feed("sudo systemctl rest\trt\r"),
            vec![("sudo systemctl restrt".to_owned(), true)],
            "Tab - дописал сервер"
        );
        assert_eq!(
            b.feed("\x1b[A\r"),
            vec![(String::new(), true)],
            "команда из истории - текст неизвестен, но факт есть"
        );
        assert_eq!(
            b.feed("\x1b[200~echo 1\recho 2\x1b[201~\r"),
            vec![("echo 1".to_owned(), false), ("echo 2".to_owned(), false)]
        );
        assert_eq!(
            b.feed("git commit -m fix wrong\x17\r"),
            vec![("git commit -m fix ".to_owned(), false)],
            "Ctrl+W - слово"
        );
    }

    #[test]
    fn строка_после_запроса_пароля_не_пишется() {
        assert!(asks_secret("user@host:~$ sudo ls\r\n[sudo] password for user: "));
        assert!(asks_secret("Enter passphrase for key '/home/u/.ssh/id_ed25519': "));
        assert!(asks_secret("\x1b[1mПароль:\x1b[0m "));
        assert!(!asks_secret("user@host:~$ "));
        assert!(
            !asks_secret("echo password\r\npassword\r\nuser@host:~$ "),
            "слово в выводе - не запрос"
        );
    }

    #[test]
    fn время_и_месячный_файл() {
        assert_eq!(
            stamp(0, 5),
            (
                "1970-01-01T00:00:00.005Z".to_owned(),
                "actions-1970-01.jsonl".to_owned()
            )
        );
        assert_eq!(stamp(1_757_931_630, 0).0, "2025-09-15T10:20:30.000Z");
        assert_eq!(
            stamp(951_782_400, 0).1,
            "actions-2000-02.jsonl",
            "29 февраля високосного года"
        );
    }

    #[test]
    fn хеш_зависит_от_предыдущей_записи_и_содержимого() {
        let body = json!({ "seq": 1, "action": "ssh.connect" });
        let a = digest("", &body);
        assert_eq!(a.len(), 64);
        assert_ne!(a, digest("x", &body), "другая предыдущая - другой хеш");
        assert_ne!(
            a,
            digest("", &json!({ "seq": 1, "action": "ssh.connec" })),
            "правка записи видна"
        );
    }

    #[test]
    fn syslog_сообщение_по_rfc_5424() {
        let m = syslog_message(
            &json!({ "t": "2026-09-15T10:00:00.000Z", "actor": { "machine": "ПК 1" }, "action": "ssh.connect" }),
        );
        assert!(
            m.starts_with("<134>1 2026-09-15T10:00:00.000Z 1 Serein - ssh.connect - {"),
            "{m}"
        );
        let cfg = syslog_cfg(
            &json!({ "actionLogSyslog": { "enabled": true, "host": " siem.local ", "port": 6514, "protocol": "tcp" } }),
        );
        assert_eq!(
            cfg,
            Some(SyslogCfg {
                host: "siem.local".into(),
                port: 6514,
                tcp: true
            })
        );
        assert_eq!(
            syslog_cfg(&json!({ "actionLogSyslog": { "enabled": true, "host": "" } })),
            None
        );
        assert_eq!(
            syslog_cfg(&json!({ "actionLogSyslog": { "enabled": false, "host": "a" } })),
            None
        );
    }

    #[test]
    fn длинный_текст_обрезается_с_пометкой() {
        let long = "я".repeat(MAX_TEXT);
        let t = text(&long);
        assert!(t.as_str().unwrap().contains("обрезано"));
        assert_eq!(text("ls"), json!("ls"));
    }
}
