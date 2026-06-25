//! Локальный терминал хоста через portable-pty (кросс-платформенный PTY).

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde_json::json;
use std::io::{Read, Write};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

pub struct LocalSession {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
}

impl LocalSession {
    pub fn write(&self, data: &str) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(data.as_bytes());
            let _ = w.flush();
        }
    }
    pub fn resize(&self, cols: u16, rows: u16) {
        if let Ok(m) = self.master.lock() {
            let _ = m.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }
    pub fn close(&self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
        }
    }
}

/// Открывает локальный shell. Поток читает вывод PTY и эмитит `session-data`,
/// по завершении — `session-exit`.
pub fn open_local(
    app: AppHandle,
    id: String,
    shell: String,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<LocalSession, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let mut cmd = CommandBuilder::new(shell);
    if let Some(dir) = cwd {
        cmd.cwd(dir);
    }
    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    // Поток-читатель вывода PTY.
    let app2 = app.clone();
    let id2 = id.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let s = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = app2.emit("session-data", json!({ "id": id2, "data": s }));
                }
                Err(_) => break,
            }
        }
        let _ = app2.emit(
            "session-exit",
            json!({ "id": id2, "code": null, "signal": null, "error": null }),
        );
    });

    Ok(LocalSession {
        writer: Mutex::new(writer),
        master: Mutex::new(pair.master),
        child: Mutex::new(child),
    })
}

/// Выбор shell для локального терминала по предпочтению (как в Electron-версии).
pub fn resolve_shell(pref: &str) -> String {
    #[cfg(windows)]
    {
        let find = |exe: &str| -> Option<String> {
            std::env::var("PATH").ok().and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|p| p.join(exe))
                    .find(|p| p.exists())
                    .map(|p| p.to_string_lossy().to_string())
            })
        };
        let win_ps = || {
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
            format!("{root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe")
        };
        if pref.contains('\\') || pref.contains('/') {
            return pref.to_string();
        }
        match pref {
            "cmd" => std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into()),
            "wsl" => find("wsl.exe").unwrap_or_else(|| "wsl.exe".into()),
            "powershell" => win_ps(),
            "pwsh" => find("pwsh.exe").unwrap_or_else(win_ps),
            _ => find("pwsh.exe").unwrap_or_else(|| {
                let ps = win_ps();
                if std::path::Path::new(&ps).exists() {
                    ps
                } else {
                    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
                }
            }),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pref;
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())
    }
}
