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

/// Как часто смотрим, не сохранил ли человек файл.
const WATCH_TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// Пауза после неудачной заливки. Правку при этом не забываем: следующий круг повторит её.
const RETRY_AFTER_FAIL: std::time::Duration = std::time::Duration::from_secs(5);

/// Запас при сравнении времени файла на сервере.
///
/// Секунда, потому что SFTP сообщает время с точностью до секунды: сравнивать точнее
/// нечем, и разница внутри секунды ничего не доказывает.
const MTIME_SLACK_MS: u64 = 1000;

/// Закрывает временный каталог от других пользователей машины.
///
/// На Linux `/tmp` общий, и файл конфигурации сервера, скачанный для правки, по умолчанию
/// доступен на чтение всем. На Windows своя папка пользователя и так закрыта списком
/// доступа, поэтому там ничего не требуется.
#[cfg(unix)]
fn close_dir(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn close_dir(_p: &Path) {}

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
        let root = std::env::temp_dir().join("serein-edit");
        let dir = root.join(uuid::Uuid::new_v4().to_string());
        // Каталоги создаём сами и сразу закрываем: скачанный сюда файл может быть
        // конфигурацией с паролями, а `/tmp` - общий.
        std::fs::create_dir_all(&dir).map_err(|e| format!("не создать временный каталог: {e}"))?;
        close_dir(&root);
        close_dir(&dir);
        let local = dir.join(&base);
        let local_str = local.to_string_lossy().to_string();

        remote_fs::download_file(&remote_fs, &handle, &remote, &local_str).await?;
        crate::store::restrict_file(&local);
        // Время правки на сервере запоминаем до начала слежки: с ним мы потом сверяемся,
        // чтобы не затереть чужую правку своей.
        let mut remote_seen = remote_fs::remote_mtime(&remote_fs, &handle, &remote)
            .await
            .ok()
            .flatten();
        let (program, args) = editor_command(&local)?;
        std::process::Command::new(&program)
            .args(&args)
            .spawn()
            .map_err(|e| format!("не удалось запустить редактор «{program}»: {e}"))?;
        emit(&app, &session_id, &remote, "opened", None);

        let running = Arc::new(AtomicBool::new(true));
        // Один наблюдатель на документ. Прежняя запись просто затиралась, и старая задача
        // продолжала жить: два наблюдателя за одним файлом заливали его по очереди,
        // каждый по своему представлению о том, что изменилось.
        if let Some(prev) = crate::sync::lock(&self.watchers)
            .insert(key(&session_id, &remote), running.clone())
        {
            prev.store(false, Ordering::Relaxed);
        }

        let mut last = mtime_of(&local);
        let remote_fs_w = remote_fs.clone();
        tokio::spawn(async move {
            // Правка, которую ещё не удалось залить. Держим её отдельно от «последнего
            // увиденного»: прежний код сдвигал отметку до отправки, поэтому сбой сети
            // означал, что правку не повторят никогда - она просто пропадала.
            let mut pending: Option<SystemTime> = None;
            loop {
                tokio::time::sleep(if pending.is_some() { RETRY_AFTER_FAIL } else { WATCH_TICK }).await;
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                let cur = mtime_of(&local);
                if let (Some(now), true) = (cur, cur != last) {
                    pending = Some(now);
                    last = cur;
                }
                let Some(_) = pending else { continue };

                // Не затираем чужое. Если файл на сервере поменялся после того, как мы его
                // скачали, заливка уничтожила бы правку, которой мы даже не видели.
                let now_remote = remote_fs::remote_mtime(&remote_fs_w, &handle, &remote)
                    .await
                    .ok()
                    .flatten();
                if let (Some(theirs), Some(ours)) = (now_remote, remote_seen) {
                    if theirs > ours + MTIME_SLACK_MS {
                        emit(
                            &app,
                            &session_id,
                            &remote,
                            "conflict",
                            Some("файл на сервере изменился - ваша правка не залита, чтобы не затереть чужую"),
                        );
                        // Ждём решения человека: сами не заливаем и не забываем правку.
                        pending = None;
                        continue;
                    }
                }

                emit(&app, &session_id, &remote, "uploading", None);
                match remote_fs::put_file(&remote_fs_w, &handle, &local_str, &remote).await {
                    Ok(_) => {
                        pending = None;
                        remote_seen = remote_fs::remote_mtime(&remote_fs_w, &handle, &remote)
                            .await
                            .ok()
                            .flatten();
                        emit(&app, &session_id, &remote, "synced", None);
                    }
                    // Правку оставляем в `pending`: следующий круг повторит её сам.
                    Err(e) => emit(&app, &session_id, &remote, "error", Some(&e)),
                }
            }
            // Уходя, забираем за собой временный каталог - но только если всё залито.
            // Несохранённую работу человека удалять нельзя ни при каких обстоятельствах.
            if pending.is_none() {
                let _ = std::fs::remove_dir_all(&dir);
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
