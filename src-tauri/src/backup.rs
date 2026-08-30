//! Зашифрованный бэкап серверов/настроек/сниппетов (порт backup.ts).
//! Формат пакета совместим с Electron-версией (crypto::encrypt_with_password).

use crate::{crypto, store};
use serde_json::{json, Value};

/// Бэкап несёт пароли серверов, поэтому пустой или совсем короткий пароль запрещаем:
/// иначе файл только выглядит зашифрованным.
const MIN_BACKUP_PASSWORD: usize = 8;

pub fn export(password: &str) -> Result<String, String> {
    if password.chars().count() < MIN_BACKUP_PASSWORD {
        return Err(format!(
            "Пароль бэкапа — минимум {MIN_BACKUP_PASSWORD} символов: файл содержит пароли серверов"
        ));
    }
    let payload = json!({
        "version": 1,
        "exportedAt": chrono_now(),
        "servers": store::list_servers_with_secrets(),
        "settings": store::settings_get(),
        "snippets": store::snippets_list(),
    });
    crypto::encrypt_with_password(&payload.to_string(), password)
}

pub fn import(content: &str, password: &str) -> Result<Value, String> {
    let json = crypto::decrypt_with_password(content, password)
        .map_err(|_| "Неверный пароль или повреждённый файл бэкапа".to_string())?;
    let payload: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    if payload.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return Err("Неподдерживаемая версия бэкапа".into());
    }
    let servers = payload.get("servers").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let snippets = payload.get("snippets").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut keys_remapped = 0usize;
    for s in &servers {
        let (s, fixed) = remap_key_path(s.clone());
        if fixed {
            keys_remapped += 1;
        }
        store::servers_save(s)?;
    }
    for s in &snippets {
        store::snippets_save(s.clone())?;
    }
    if let Some(settings) = payload.get("settings") {
        store::settings_set(settings.clone())?;
    }
    Ok(json!({
        "servers": servers.len(),
        "snippets": snippets.len(),
        "keysRemapped": keys_remapped,
    }))
}

/// Подставить в профиль путь к ключу, который существует на этой системе.
///
/// Бэкап с Windows несёт абсолютный путь вида `C:\Users\…\.ssh\id_ed25519`. На Linux
/// такого файла нет, и подключение падало на ровном месте. Правим только когда файл
/// действительно нашёлся: переписать путь на другой, столь же несуществующий, — значит
/// соврать пользователю и спрятать причину.
fn remap_key_path(mut s: Value) -> (Value, bool) {
    let Some(o) = s.as_object_mut() else {
        return (s, false);
    };
    let Some(raw) = o.get("privateKeyPath").and_then(|v| v.as_str()).map(|x| x.to_string()) else {
        return (s, false);
    };
    if raw.trim().is_empty() {
        return (s, false);
    }
    let fixed = crate::paths::resolve_identity(&raw);
    if fixed != raw && std::path::Path::new(&fixed).exists() {
        o.insert("privateKeyPath".into(), json!(fixed));
        return (s, true);
    }
    (s, false)
}

// Лёгкая ISO-метка без зависимости от chrono.
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("@{secs}")
}
