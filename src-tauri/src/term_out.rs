//! Terminal output batching: fewer Tauri events; SSH/PTY reader never waits on IPC.
//!
//! Producers only append bytes (short Mutex). A worker flushes ~once per frame or at
//! BATCH_BYTES. If the UI lags, oldest pending bytes are dropped so the tail stays visible.

use serde_json::json;
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

const BATCH_BYTES: usize = 48 * 1024;
const BATCH_MS: u64 = 16;
const MAX_PENDING: usize = 4 * 1024 * 1024;

struct State {
    buf: Vec<u8>,
    dropped: u64,
    closed: bool,
}

#[derive(Clone)]
pub struct TermOut {
    st: Arc<Mutex<State>>,
    notify: Arc<Notify>,
    emits: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
}

impl TermOut {
    pub fn spawn(app: AppHandle, id: String) -> Self {
        let out = Self {
            st: Arc::new(Mutex::new(State {
                buf: Vec::with_capacity(BATCH_BYTES),
                dropped: 0,
                closed: false,
            })),
            notify: Arc::new(Notify::new()),
            emits: Arc::new(AtomicU64::new(0)),
            bytes: Arc::new(AtomicU64::new(0)),
            finished: Arc::new(AtomicBool::new(false)),
        };
        let worker = out.clone();
        tauri::async_runtime::spawn(async move { worker.run(app, id).await });
        out
    }

    pub fn push(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut g = self.st.lock().expect("term_out");
        if g.closed {
            return;
        }
        g.buf.extend_from_slice(bytes);
        if g.buf.len() > MAX_PENDING {
            let overflow = g.buf.len() - MAX_PENDING;
            g.buf.drain(..overflow);
            g.dropped += overflow as u64;
        }
        drop(g);
        self.notify.notify_one();
    }

    pub fn close(&self) {
        if let Ok(mut g) = self.st.lock() {
            g.closed = true;
        }
        self.notify.notify_one();
    }

    pub async fn join(&self) {
        let start = Instant::now();
        while !self.finished.load(Ordering::Acquire) && start.elapsed() < Duration::from_millis(250) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    pub fn join_blocking(&self) {
        let start = Instant::now();
        while !self.finished.load(Ordering::Acquire) && start.elapsed() < Duration::from_millis(250) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    async fn run(self, app: AppHandle, id: String) {
        let stats = std::env::var_os("SEREIN_TERM_PIPE").is_some();
        let mut last_stats = Instant::now();
        loop {
            let (empty, closed) = {
                let g = self.st.lock().expect("term_out");
                (g.buf.is_empty(), g.closed)
            };
            if empty && closed {
                break;
            }
            if empty {
                self.notify.notified().await;
                continue;
            }
            let deadline = tokio::time::Instant::now() + Duration::from_millis(BATCH_MS);
            loop {
                let n = self.st.lock().expect("term_out").buf.len();
                if n >= BATCH_BYTES || n == 0 {
                    break;
                }
                tokio::select! {
                    _ = self.notify.notified() => {}
                    _ = tokio::time::sleep_until(deadline) => break,
                }
            }
            let (chunk, dropped) = self.take();
            if dropped > 0 {
                let note = format!(
                    "\r\n\x1b[33m\u{447}\u{430}\u{441}\u{442}\u{44c} \u{432}\u{44b}\u{432}\u{43e}\u{434}\u{430} \u{43f}\u{440}\u{43e}\u{43f}\u{443}\u{449}\u{435}\u{43d}\u{430} ({})\x1b[0m\r\n",
                    fmt_bytes(dropped)
                );
                let _ = app.emit("session-data", json!({ "id": id, "data": note }));
            }
            if !chunk.is_empty() {
                self.emits.fetch_add(1, Ordering::Relaxed);
                self.bytes.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                log_write(&id, &chunk);
                let _ = app.emit("session-data", json!({ "id": id, "data": chunk }));
            }
            if stats && last_stats.elapsed() >= Duration::from_secs(2) {
                let e = self.emits.swap(0, Ordering::Relaxed);
                let b = self.bytes.swap(0, Ordering::Relaxed);
                eprintln!("[term_pipe] {id} emits={e} bytes={b}/2s");
                last_stats = Instant::now();
            }
        }
        log_stop(&id);
        self.finished.store(true, Ordering::Release);
    }

    fn take(&self) -> (String, u64) {
        let mut g = self.st.lock().expect("term_out");
        let dropped = std::mem::take(&mut g.dropped);
        let s = take_utf8(&mut g.buf);
        (s, dropped)
    }
}

fn fmt_bytes(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} \u{41c}\u{411}", n as f64 / (1024.0 * 1024.0))
    } else if n >= 1024 {
        format!("{} \u{41a}\u{411}", n / 1024)
    } else {
        format!("{n} \u{411}")
    }
}

/// Complete UTF-8 chars only; incomplete tail stays in the buffer for the next flush.
fn take_utf8(buf: &mut Vec<u8>) -> String {
    if buf.is_empty() {
        return String::new();
    }
    match std::str::from_utf8(buf) {
        Ok(_) => String::from_utf8(std::mem::take(buf)).unwrap_or_default(),
        Err(e) => {
            let valid = e.valid_up_to();
            if e.error_len().is_some() {
                let skip = e.error_len().unwrap_or(1);
                let s = String::from_utf8_lossy(&buf[..valid]).into_owned();
                let drain = (valid + skip).min(buf.len());
                buf.drain(..drain);
                s
            } else if valid == 0 {
                String::new()
            } else {
                let s = String::from_utf8_lossy(&buf[..valid]).into_owned();
                buf.drain(..valid);
                s
            }
        }
    }
}

// ---------- Логирование сессии в файл ----------
//
// Точка одна для SSH и локального терминала: пишем то, что уходит в xterm, но без
// ANSI-кодов — иначе лог невозможно читать глазами. Состояние разбора escape-последо-
// вательностей живёт между пачками, потому что пачка может оборваться посреди кода.

struct LogSink {
    file: std::fs::File,
    path: String,
    esc: Esc,
}

#[derive(Clone, Copy, PartialEq)]
enum Esc {
    None,
    Start,
    Csi,
    Osc,
    OscEsc,
}

fn logs() -> &'static Mutex<HashMap<String, LogSink>> {
    static LOGS: OnceLock<Mutex<HashMap<String, LogSink>>> = OnceLock::new();
    LOGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn strip_ansi_into(state: &mut Esc, s: &str, out: &mut String) {
    for ch in s.chars() {
        match *state {
            Esc::None => match ch {
                '\x1b' => *state = Esc::Start,
                '\r' => {}
                '\n' | '\t' => out.push(ch),
                c if !c.is_control() => out.push(c),
                _ => {}
            },
            Esc::Start => {
                *state = match ch {
                    '[' => Esc::Csi,
                    ']' => Esc::Osc,
                    _ => Esc::None,
                }
            }
            Esc::Csi => {
                if ('\x40'..='\x7e').contains(&ch) {
                    *state = Esc::None;
                }
            }
            Esc::Osc => match ch {
                '\x07' => *state = Esc::None,
                '\x1b' => *state = Esc::OscEsc,
                _ => {}
            },
            Esc::OscEsc => *state = Esc::None,
        }
    }
}

/// Дата-время UTC как `YYYYMMDD-HHMMSS` без зависимости от chrono.
fn stamp_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Алгоритм Хиннанта: дни от эпохи → григорианская дата.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Имя файла из заголовка вкладки: только то, что Windows примет как имя файла.
fn safe_name(title: &str) -> String {
    let mut cleaned = String::new();
    for c in title.chars() {
        if c.is_alphanumeric() || c == '_' || c == '.' {
            cleaned.push(c);
        } else if !cleaned.ends_with('-') {
            cleaned.push('-'); // пробелы, «@», «:» и слэши схлопываем в один дефис
        }
    }
    let short: String = cleaned.trim_matches('-').chars().take(48).collect();
    let short = short.trim_end_matches('-');
    if short.is_empty() {
        "session".into()
    } else {
        short.to_string()
    }
}

/// Включает запись лога сессии. Возвращает путь к файлу.
pub fn log_start(id: &str, title: &str) -> Result<String, String> {
    let dir = crate::store::config_dir().join("logs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}-{}.log", safe_name(title), stamp_utc()));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let header = format!("# Serein — лог сессии «{title}», начат {} UTC\n", stamp_utc());
    file.write_all(header.as_bytes()).map_err(|e| e.to_string())?;
    let path_str = path.to_string_lossy().to_string();
    logs().lock().map_err(|_| "Реестр логов занят")?.insert(
        id.to_string(),
        LogSink { file, path: path_str.clone(), esc: Esc::None },
    );
    Ok(path_str)
}

/// Выключает запись. `true`, если лог действительно писался.
pub fn log_stop(id: &str) -> bool {
    match logs().lock() {
        Ok(mut g) => g.remove(id).is_some(),
        Err(_) => false,
    }
}

pub fn log_active(id: &str) -> bool {
    logs().lock().map(|g| g.contains_key(id)).unwrap_or(false)
}

pub fn log_path_of(id: &str) -> Option<String> {
    logs().lock().ok()?.get(id).map(|s| s.path.clone())
}

fn log_write(id: &str, chunk: &str) {
    let Ok(mut g) = logs().lock() else { return };
    let Some(sink) = g.get_mut(id) else { return };
    let mut text = String::with_capacity(chunk.len());
    strip_ansi_into(&mut sink.esc, chunk, &mut text);
    if text.is_empty() {
        return;
    }
    // Сессию не рвём из-за проблемы с диском: просто прекращаем писать.
    if sink.file.write_all(text.as_bytes()).is_err() {
        g.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::{safe_name, strip_ansi_into, Esc};

    #[test]
    fn strips_colors_and_keeps_text() {
        let mut st = Esc::None;
        let mut out = String::new();
        strip_ansi_into(&mut st, "\x1b[32mhade@srv\x1b[0m:~$ ls\r\n", &mut out);
        assert_eq!(out, "hade@srv:~$ ls\n");
    }

    #[test]
    fn escape_split_across_chunks() {
        let mut st = Esc::None;
        let mut out = String::new();
        strip_ansi_into(&mut st, "ab\x1b[3", &mut out);
        strip_ansi_into(&mut st, "1mcd", &mut out);
        assert_eq!(out, "abcd");
    }

    #[test]
    fn strips_osc_title() {
        let mut st = Esc::None;
        let mut out = String::new();
        strip_ansi_into(&mut st, "\x1b]0;заголовок\x07готово", &mut out);
        assert_eq!(out, "готово");
    }

    #[test]
    fn file_name_is_safe() {
        assert_eq!(safe_name("hade@192.168.0.156: /srv"), "hade-192.168.0.156-srv");
        assert_eq!(safe_name("Локальный терминал"), "Локальный-терминал");
        assert_eq!(safe_name("///"), "session");
        assert_eq!(safe_name(""), "session");
    }
}
