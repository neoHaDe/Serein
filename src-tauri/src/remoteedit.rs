//! Редактирование удалённого файла во внешнем редакторе (порт remoteEdit.ts).
//! Скачиваем во временный файл, открываем в редакторе ОС, следим за mtime и
//! заливаем обратно при изменении, эмитя статус в renderer.

use crate::remote_fs;
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

/// Редакторы, которыми открываем файл, если в настройках ничего не выбрано.
///
/// Для Linux - список: единого «блокнота» там нет, а ставить зависимость от рабочего стола
/// ради одной кнопки незачем. Берём первый, который найдётся в `PATH`.
#[cfg(target_os = "linux")]
const LINUX_EDITORS: &[&str] = &[
    "gnome-text-editor",
    "gedit",
    "kate",
    "kwrite",
    "mousepad",
    "xed",
    "pluma",
    "leafpad",
];

/// Есть ли такая программа в `PATH`.
#[cfg(target_os = "linux")]
fn in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(name).is_file()))
        .unwrap_or(false)
}

/// Чем и как открыть скачанный файл.
///
/// Ассоциацией ОС открывать нельзя, и это не теоретическая придирка: у скачанного
/// `.exe`, `.bat`, `.cmd` или `.ps1` ассоциация - «запустить». Человек нажимал
/// «Открыть в редакторе», а получал запуск чужого файла со своими правами.
///
/// Поэтому запускаем программу-редактор, а путь отдаём ей отдельным доводом: без оболочки,
/// без ассоциаций, и пробелы с кавычками в имени при этом ничего не значат.
fn editor_command(local: &Path) -> Result<(String, Vec<String>), String> {
    let chosen = crate::store::settings_get()
        .get("externalEditor")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    editor_for(&chosen, local)
}

/// То же решение без настроек и диска - чтобы его можно было проверить.
fn editor_for(chosen: &str, local: &Path) -> Result<(String, Vec<String>), String> {
    let chosen = chosen.trim();
    let path = local.to_string_lossy().to_string();
    if !chosen.is_empty() {
        return Ok((chosen.to_owned(), vec![path]));
    }
    #[cfg(windows)]
    {
        Ok(("notepad.exe".to_owned(), vec![path]))
    }
    #[cfg(target_os = "macos")]
    {
        // `-e` - именно текстовый редактор системы, а не «чем принято открывать такое».
        Ok(("open".to_owned(), vec!["-e".to_owned(), path]))
    }
    #[cfg(target_os = "linux")]
    {
        match LINUX_EDITORS.iter().find(|e| in_path(e)) {
            Some(e) => Ok(((*e).to_owned(), vec![path])),
            None => Err(
                "не нашёл текстовый редактор - укажите его в настройках, в поле «Внешний редактор»"
                    .to_owned(),
            ),
        }
    }
}

impl EditManager {
    pub async fn open(
        &self,
        app: AppHandle,
        handle: Arc<tokio::sync::Mutex<client::Handle<crate::ssh::ClientHandler>>>,
        remote_fs: Arc<Mutex<remote_fs::SessionFs>>,
        session_id: String,
        remote: String,
    ) -> Result<(), String> {
        let base = Path::new(&remote)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        // Имя пришло с сервера, а из него собирается путь на своей машине.
        crate::localname::safe_component(&base)
            .map_err(|why| format!("не могу сохранить для правки: {why}"))?;
        let dir = std::env::temp_dir()
            .join("terminal-edit")
            .join(uuid::Uuid::new_v4().to_string());
        let local = dir.join(&base);
        let local_str = local.to_string_lossy().to_string();

        remote_fs::download_file(&remote_fs, &handle, &remote, &local_str).await?;
        let (program, args) = editor_command(&local)?;
        std::process::Command::new(&program)
            .args(&args)
            .spawn()
            .map_err(|e| format!("не удалось запустить редактор «{program}»: {e}"))?;
        emit(&app, &session_id, &remote, "opened", None);

        let running = Arc::new(AtomicBool::new(true));
        crate::sync::lock(&self.watchers).insert(key(&session_id, &remote), running.clone());

        let mut last = mtime_of(&local);
        let remote_fs_w = remote_fs.clone();
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
                    match remote_fs::put_file(&remote_fs_w, &handle, &local_str, &remote).await {
                        Ok(_) => emit(&app, &session_id, &remote, "synced", None),
                        Err(e) => emit(&app, &session_id, &remote, "error", Some(&e)),
                    }
                }
            }
        });
        Ok(())
    }

    pub fn stop(&self, app: &AppHandle, session_id: &str, remote: &str) {
        if let Some(r) = crate::sync::lock(&self.watchers).remove(&key(session_id, remote)) {
            r.store(false, Ordering::Relaxed);
        }
        emit(app, session_id, remote, "stopped", None);
    }

    pub fn stop_session(&self, session_id: &str) {
        let mut w = crate::sync::lock(&self.watchers);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn путь_уходит_редактору_отдельным_доводом() {
        // Один довод - один путь. Ни оболочки, ни склейки строк: иначе имя с пробелом или
        // кавычкой превращается в несколько доводов, а то и в команду.
        let file = Path::new("/tmp/папка с пробелом/от\"чёт.txt");
        let (program, args) = editor_for("  /usr/bin/gedit ", file).expect("редактор выбран");
        assert_eq!(program, "/usr/bin/gedit", "пробелы вокруг пути к программе не значат ничего");
        assert_eq!(args, vec![file.to_string_lossy().to_string()]);
    }

    #[test]
    fn без_настройки_берётся_текстовый_редактор_системы() {
        // Главное здесь - чего в ответе НЕТ: ассоциации ОС. У скачанного `.exe` она
        // означает «запустить», и кнопка «открыть в редакторе» запускала бы чужой файл.
        let exe = Path::new("/tmp/загрузка/вирус.exe");
        match editor_for("", exe) {
            Ok((program, args)) => {
                assert!(
                    !program.to_ascii_lowercase().ends_with("exe")
                        || program.to_ascii_lowercase().ends_with("notepad.exe"),
                    "запускать надо редактор, а не сам файл: {program}"
                );
                assert!(
                    args.iter().any(|a| a.ends_with("вирус.exe")),
                    "файл отдаётся доводом: {args:?}"
                );
            }
            // На Linux без единого установленного редактора выбирать нечего - и тогда мы
            // говорим об этом, а не открываем файл ассоциацией.
            Err(why) => assert!(why.contains("редактор"), "{why}"),
        }
    }
}
