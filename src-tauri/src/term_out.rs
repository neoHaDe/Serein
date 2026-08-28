//! Terminal output batching: fewer Tauri events; SSH/PTY reader never waits on IPC.
//!
//! Producers only append bytes (short Mutex). A worker flushes ~once per frame or at
//! BATCH_BYTES. If the UI lags, oldest pending bytes are dropped so the tail stays visible.

use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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
                let _ = app.emit("session-data", json!({ "id": id, "data": chunk }));
            }
            if stats && last_stats.elapsed() >= Duration::from_secs(2) {
                let e = self.emits.swap(0, Ordering::Relaxed);
                let b = self.bytes.swap(0, Ordering::Relaxed);
                eprintln!("[term_pipe] {id} emits={e} bytes={b}/2s");
                last_stats = Instant::now();
            }
        }
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
