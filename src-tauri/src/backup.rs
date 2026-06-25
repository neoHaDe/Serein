//! Зашифрованный бэкап серверов/настроек/сниппетов (порт backup.ts).
//! Формат пакета совместим с Electron-версией (crypto::encrypt_with_password).

use crate::{crypto, store};
use serde_json::{json, Value};

pub fn export(password: &str) -> Result<String, String> {
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
    for s in &servers {
        store::servers_save(s.clone())?;
    }
    for s in &snippets {
        store::snippets_save(s.clone())?;
    }
    if let Some(settings) = payload.get("settings") {
        store::settings_set(settings.clone())?;
    }
    Ok(json!({ "servers": servers.len(), "snippets": snippets.len() }))
}

// Лёгкая ISO-метка без зависимости от chrono.
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("@{secs}")
}
