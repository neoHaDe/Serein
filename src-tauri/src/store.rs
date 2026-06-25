//! Простое JSON-хранилище в каталоге конфигурации (серверы/настройки/сниппеты/раскладка).
//! Значения храним как `serde_json::Value`, чтобы не мирроровать все типы фронтенда.

use crate::{crypto, dpapi, vaultkey};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

fn dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let d = base.join("term-tauri");
    let _ = fs::create_dir_all(&d);
    d
}

fn read_value(name: &str) -> Option<Value> {
    let txt = fs::read_to_string(dir().join(name)).ok()?;
    serde_json::from_str(&txt).ok()
}

fn write_value(name: &str, v: &Value) -> Result<(), String> {
    let txt = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    fs::write(dir().join(name), txt).map_err(|e| e.to_string())
}

// ---------- Настройки ----------

fn default_settings() -> Value {
    json!({
        "theme": "Tokyo Night",
        "fontSize": 14,
        "fontFamily": "Cascadia Code, Consolas, \"Courier New\", monospace",
        "openLocalOnStart": false,
        "autoReconnect": false,
        "sidebarWidth": 270,
        "sftpWidth": 380,
        "keybindings": {},
        "restoreTabsOnStart": false,
        "localShell": "auto",
        "density": "comfortable"
    })
}

pub fn settings_get() -> Value {
    let mut base = default_settings();
    if let (Some(b), Some(stored)) = (base.as_object_mut(), read_value("settings.json")) {
        if let Some(s) = stored.as_object() {
            for (k, v) in s {
                b.insert(k.clone(), v.clone());
            }
        }
    }
    base
}

pub fn settings_set(patch: Value) -> Result<Value, String> {
    let mut cur = settings_get();
    if let (Some(obj), Some(p)) = (cur.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            obj.insert(k.clone(), v.clone());
        }
    }
    write_value("settings.json", &cur)?;
    Ok(cur)
}

// ---------- Универсальный список объектов с полем id ----------

fn list_items(name: &str) -> Vec<Value> {
    read_value(name)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

fn upsert_item(name: &str, mut item: Value) -> Result<Value, String> {
    let id = item
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    if let Some(obj) = item.as_object_mut() {
        obj.insert("id".into(), Value::String(id.clone()));
    }
    let mut items = list_items(name);
    if let Some(pos) = items
        .iter()
        .position(|i| i.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
    {
        items[pos] = item.clone();
    } else {
        items.push(item.clone());
    }
    write_value(name, &Value::Array(items))?;
    Ok(item)
}

fn delete_item(name: &str, id: &str) -> Result<(), String> {
    let items: Vec<Value> = list_items(name)
        .into_iter()
        .filter(|i| i.get("id").and_then(|v| v.as_str()) != Some(id))
        .collect();
    write_value(name, &Value::Array(items))
}

// ---------- Серверы ----------

// ----- Шифрование секретов (DPAPI + опциональный мастер-слой scrypt→AES) -----

fn os_protect(v: &str) -> Option<String> {
    #[cfg(windows)]
    {
        dpapi::protect(v.as_bytes()).ok().map(|b| STANDARD.encode(b))
    }
    #[cfg(not(windows))]
    {
        Some(format!("plain:{}", STANDARD.encode(v.as_bytes())))
    }
}

fn encrypt_secret(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    // Доп. слой мастер-пароля поверх DPAPI, если задан и разблокирован.
    let v = match vaultkey::get() {
        Some(mk) => format!("mk:{}", crypto::aes_encrypt(value, &mk).ok()?),
        None => value.to_string(),
    };
    os_protect(&v)
}

fn decrypt_secret(enc: &str) -> Option<String> {
    let v = if let Some(rest) = enc.strip_prefix("plain:") {
        String::from_utf8(STANDARD.decode(rest).ok()?).ok()?
    } else {
        let bytes = STANDARD.decode(enc).ok()?;
        String::from_utf8(dpapi::unprotect(&bytes).ok()?).ok()?
    };
    if let Some(rest) = v.strip_prefix("mk:") {
        let mk = vaultkey::get()?; // заблокировано — секрет недоступен
        crypto::aes_decrypt(rest, &mk).ok()
    } else {
        Some(v)
    }
}

fn read_secrets() -> Value {
    read_value("secrets.json").unwrap_or_else(|| json!({}))
}

pub fn servers_list() -> Vec<Value> {
    list_items("servers.json")
}
/// Список серверов БЕЗ секретов — для UI.
pub fn servers_list_safe() -> Vec<Value> {
    servers_list()
        .into_iter()
        .map(|mut s| {
            if let Some(o) = s.as_object_mut() {
                o.remove("password");
                o.remove("passphrase");
            }
            s
        })
        .collect()
}

pub fn servers_save(mut cfg: Value) -> Result<Value, String> {
    let password = cfg.get("password").and_then(|v| v.as_str()).map(|s| s.to_string());
    let passphrase = cfg.get("passphrase").and_then(|v| v.as_str()).map(|s| s.to_string());
    let had_password = cfg.get("password").is_some();
    let had_passphrase = cfg.get("passphrase").is_some();
    if let Some(o) = cfg.as_object_mut() {
        o.remove("password");
        o.remove("passphrase");
    }
    let base = upsert_item("servers.json", cfg)?;
    let id = base.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();

    // Секреты: заданное значение перезаписывает, отсутствие ключа — сохраняет прежнее.
    let mut secrets = read_secrets();
    let prev = secrets.get(&id).cloned().unwrap_or_else(|| json!({}));
    let mut next = Map::new();
    let pw = if had_password {
        password.as_deref().and_then(encrypt_secret).map(Value::String)
    } else {
        prev.get("password").cloned()
    };
    let pp = if had_passphrase {
        passphrase.as_deref().and_then(encrypt_secret).map(Value::String)
    } else {
        prev.get("passphrase").cloned()
    };
    if let Some(v) = pw {
        next.insert("password".into(), v);
    }
    if let Some(v) = pp {
        next.insert("passphrase".into(), v);
    }
    if let Some(o) = secrets.as_object_mut() {
        o.insert(id.clone(), Value::Object(next));
    }
    write_value("secrets.json", &secrets)?;
    Ok(base)
}

pub fn servers_delete(id: &str) -> Result<(), String> {
    delete_item("servers.json", id)?;
    let mut secrets = read_secrets();
    if let Some(o) = secrets.as_object_mut() {
        o.remove(id);
    }
    write_value("secrets.json", &secrets)
}

/// Полный конфиг сервера ВМЕСТЕ с расшифрованными секретами — только для подключения.
pub fn server_with_secrets(id: &str) -> Option<Value> {
    let mut base = servers_list()
        .into_iter()
        .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(id))?;
    let secrets = read_secrets();
    if let Some(sec) = secrets.get(id) {
        if let Some(o) = base.as_object_mut() {
            if let Some(enc) = sec.get("password").and_then(|v| v.as_str()) {
                if let Some(p) = decrypt_secret(enc) {
                    o.insert("password".into(), Value::String(p));
                }
            }
            if let Some(enc) = sec.get("passphrase").and_then(|v| v.as_str()) {
                if let Some(p) = decrypt_secret(enc) {
                    o.insert("passphrase".into(), Value::String(p));
                }
            }
        }
    }
    Some(base)
}

/// Все серверы с расшифрованными секретами (для бэкапа).
pub fn list_servers_with_secrets() -> Vec<Value> {
    servers_list()
        .into_iter()
        .filter_map(|s| s.get("id").and_then(|v| v.as_str()).map(|id| id.to_string()))
        .filter_map(|id| server_with_secrets(&id))
        .collect()
}

/// Расшифровать все секреты (для re-wrap мастер-ключом).
pub fn export_all_secrets() -> Map<String, Value> {
    let secrets = read_secrets();
    let mut out = Map::new();
    if let Some(obj) = secrets.as_object() {
        for (id, sec) in obj {
            let pw = sec.get("password").and_then(|v| v.as_str()).and_then(decrypt_secret);
            let pp = sec.get("passphrase").and_then(|v| v.as_str()).and_then(decrypt_secret);
            out.insert(id.clone(), json!({ "password": pw, "passphrase": pp }));
        }
    }
    out
}

/// Перешифровать секреты при ТЕКУЩЕМ состоянии ключа и записать.
pub fn import_all_secrets(map: &Map<String, Value>) -> Result<(), String> {
    let mut secrets = read_secrets();
    if let Some(obj) = secrets.as_object_mut() {
        for (id, pair) in map {
            let mut next = Map::new();
            if let Some(pw) = pair.get("password").and_then(|v| v.as_str()).and_then(encrypt_secret) {
                next.insert("password".into(), Value::String(pw));
            }
            if let Some(pp) = pair.get("passphrase").and_then(|v| v.as_str()).and_then(encrypt_secret) {
                next.insert("passphrase".into(), Value::String(pp));
            }
            obj.insert(id.clone(), Value::Object(next));
        }
    }
    write_value("secrets.json", &secrets)
}


// ---------- Сниппеты ----------

pub fn snippets_list() -> Vec<Value> {
    list_items("snippets.json")
}
pub fn snippets_save(s: Value) -> Result<Value, String> {
    upsert_item("snippets.json", s)
}
pub fn snippets_delete(id: &str) -> Result<(), String> {
    delete_item("snippets.json", id)
}

// ---------- Раскладка вкладок ----------

pub fn layout_get() -> Value {
    read_value("layout.json").unwrap_or_else(|| Value::Array(vec![]))
}
pub fn layout_set(tabs: Value) -> Result<(), String> {
    write_value("layout.json", &tabs)
}

// Заглушка, чтобы избежать предупреждения о неиспользуемом импорте Map в некоторых конфигурациях.
#[allow(dead_code)]
fn _unused(_m: Map<String, Value>) {}
