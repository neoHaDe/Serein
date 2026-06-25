//! Редактирование удалённого файла во внешнем редакторе (порт remoteEdit.ts).
//! Скачиваем во временный файл, открываем в редакторе ОС, следим за mtime и
//! заливаем обратно при изменении, эмитя статус в renderer.

use crate::sftp;
use crate::ssh::ClientHandler;
use russh::client;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tauri::{AppHandle, Emitter};

#[derive(Default)]
pub struct EditManager {
    watchers: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

fn key(session_id: &str, remote: &str) -> String {
    format!("{session_id}\u{0}{remote}")
}

fn emit(app: &AppHandle, session_id: &str, remote: &str, state: &str, error: Option<&str>) {
    let _ = app.emit(
        "sftp-edit-status",
        json!({ "sessionId": session_id, "remotePath": remote, "state": state, "error": error }),
    );
}

fn mtime_of(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok().and_then(|m| m.modified().ok())
}

impl EditManager {
    pub async fn open(
        &self,
        app: AppHandle,
        handle: Arc<tokio::sync::Mutex<client::Handle<ClientHandler>>>,
        session_id: String,
        remote: String,
    ) -> Result<(), String> {
        let base = Path::new(&remote)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        let dir = std::env::temp_dir()
            .join("terminal-edit")
            .join(uuid::Uuid::new_v4().to_string());
        let local = dir.join(&base);
        let local_str = local.to_string_lossy().to_string();

        sftp::download_file(&handle, &remote, &local_str).await?;
        open::that(&local).map_err(|e| format!("Не удалось открыть редактор: {e}"))?;
        emit(&app, &session_id, &remote, "opened", None);

        let running = Arc::new(AtomicBool::new(true));
        self.watchers.lock().unwrap().insert(key(&session_id, &remote), running.clone());

        let mut last = mtime_of(&local);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                let cur = mtime_of(&local);
                if cur != last && cur.is_some() {
                    last = cur;
                    emit(&app, &session_id, &remote, "uploading", None);
                    match sftp::put_file(&handle, &local_str, &remote).await {
                        Ok(_) => emit(&app, &session_id, &remote, "synced", None),
                        Err(e) => emit(&app, &session_id, &remote, "error", Some(&e)),
                    }
                }
            }
        });
        Ok(())
    }

    pub fn stop(&self, app: &AppHandle, session_id: &str, remote: &str) {
        if let Some(r) = self.watchers.lock().unwrap().remove(&key(session_id, remote)) {
            r.store(false, Ordering::Relaxed);
        }
        emit(app, session_id, remote, "stopped", None);
    }

    pub fn stop_session(&self, session_id: &str) {
        let mut w = self.watchers.lock().unwrap();
        let keys: Vec<String> = w
            .keys()
            .filter(|k| k.starts_with(&format!("{session_id}\u{0}")))
            .cloned()
            .collect();
        for k in keys {
            if let Some(r) = w.remove(&k) {
                r.store(false, Ordering::Relaxed);
            }
        }
    }
}
