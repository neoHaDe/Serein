//! Опциональный мастер-пароль (порт vault.ts): scrypt-ключ доп. слоем поверх DPAPI.

use crate::{crypto, store, vaultkey};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::path::PathBuf;

const VERIFY_TOKEN: &str = "TERMINAL_VAULT_OK";

fn vault_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("term-tauri")
        .join("vault.json")
}

fn read_config() -> Option<Value> {
    let txt = std::fs::read_to_string(vault_path()).ok()?;
    let cfg: Value = serde_json::from_str(&txt).ok()?;
    if cfg.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
        Some(cfg)
    } else {
        None
    }
}

pub fn is_enabled() -> bool {
    read_config().is_some()
}

pub fn status() -> Value {
    let enabled = is_enabled();
    json!({ "enabled": enabled, "locked": enabled && vaultkey::get().is_none() })
}

fn key_from(password: &str, salt_b64: &str) -> Option<[u8; 32]> {
    let salt = STANDARD.decode(salt_b64).ok()?;
    Some(crypto::derive_key(password, &salt))
}

pub fn unlock(password: &str) -> bool {
    let Some(cfg) = read_config() else { return false };
    let salt = cfg.get("salt").and_then(|v| v.as_str()).unwrap_or("");
    let verifier = cfg.get("verifier").and_then(|v| v.as_str()).unwrap_or("");
    let Some(key) = key_from(password, salt) else { return false };
    match crypto::aes_decrypt(verifier, &key) {
        Ok(t) if t == VERIFY_TOKEN => {
            vaultkey::set(Some(key));
            true
        }
        _ => false,
    }
}

pub fn enable(password: &str) -> Value {
    if is_enabled() {
        return json!({ "ok": false, "error": "Мастер-пароль уже включён" });
    }
    if password.len() < 4 {
        return json!({ "ok": false, "error": "Пароль слишком короткий (мин. 4 символа)" });
    }
    // Вынимаем секреты при текущем (пустом) ключе, затем перешифровываем с мастер-слоем.
    let plain = store::export_all_secrets();
    let salt = {
        use rand::RngCore;
        let mut s = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut s);
        s
    };
    let key = crypto::derive_key(password, &salt);
    vaultkey::set(Some(key));
    if let Err(e) = store::import_all_secrets(&plain) {
        return json!({ "ok": false, "error": e });
    }
    let verifier = match crypto::aes_encrypt(VERIFY_TOKEN, &key) {
        Ok(v) => v,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let cfg = json!({ "enabled": true, "salt": STANDARD.encode(salt), "verifier": verifier });
    if let Err(e) = std::fs::write(vault_path(), serde_json::to_string_pretty(&cfg).unwrap()) {
        return json!({ "ok": false, "error": e.to_string() });
    }
    json!({ "ok": true })
}

pub fn disable(password: &str) -> Value {
    let Some(cfg) = read_config() else {
        return json!({ "ok": false, "error": "Мастер-пароль не задан" });
    };
    let salt = cfg.get("salt").and_then(|v| v.as_str()).unwrap_or("");
    let verifier = cfg.get("verifier").and_then(|v| v.as_str()).unwrap_or("");
    let Some(key) = key_from(password, salt) else {
        return json!({ "ok": false, "error": "Неверный пароль" });
    };
    if crypto::aes_decrypt(verifier, &key).ok().as_deref() != Some(VERIFY_TOKEN) {
        return json!({ "ok": false, "error": "Неверный пароль" });
    }
    vaultkey::set(Some(key));
    let plain = store::export_all_secrets();
    vaultkey::set(None);
    if let Err(e) = store::import_all_secrets(&plain) {
        return json!({ "ok": false, "error": e });
    }
    let _ = std::fs::remove_file(vault_path());
    json!({ "ok": true })
}
