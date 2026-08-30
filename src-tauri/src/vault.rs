//! Опциональный мастер-пароль (порт vault.ts): scrypt-ключ доп. слоем поверх DPAPI.

use crate::{crypto, vaultkey};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::path::PathBuf;

const VERIFY_TOKEN: &str = "TERMINAL_VAULT_OK";

fn vault_path() -> PathBuf {
    crate::store::config_dir().join("vault.json")
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

/// Параметры KDF из конфигурации хранилища.
///
/// Их не было в прежних сборках, поэтому отсутствие поля означает набор 2009 года.
/// Существующее хранилище на новые параметры молча не переводим: ключ изменился бы,
/// и все уже зашифрованные секреты перестали бы открываться. Новые параметры
/// достаются при следующем включении мастер-пароля — там секреты и так перешифровываются.
fn kdf_of(cfg: &Value) -> crypto::Kdf {
    let Some(k) = cfg.get("kdf") else {
        return crypto::KDF_LEGACY;
    };
    let num = |name: &str, d: u32| k.get(name).and_then(|v| v.as_u64()).unwrap_or(d as u64) as u32;
    crypto::Kdf {
        log_n: num("logN", crypto::KDF_LEGACY.log_n as u32) as u8,
        r: num("r", crypto::KDF_LEGACY.r),
        p: num("p", crypto::KDF_LEGACY.p),
    }
}

fn key_from(password: &str, salt_b64: &str, kdf: crypto::Kdf) -> Option<[u8; 32]> {
    let salt = STANDARD.decode(salt_b64).ok()?;
    Some(crypto::derive_key_with(password, &salt, kdf))
}

pub fn unlock(password: &str) -> bool {
    let Some(cfg) = read_config() else { return false };
    let salt = cfg.get("salt").and_then(|v| v.as_str()).unwrap_or("");
    let verifier = cfg.get("verifier").and_then(|v| v.as_str()).unwrap_or("");
    let Some(key) = key_from(password, salt, kdf_of(&cfg)) else { return false };
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
    let plain = crate::store::export_all_secrets();
    let salt = {
        use rand::RngCore;
        let mut s = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut s);
        s
    };
    let kdf = crypto::KDF_CURRENT;
    let key = crypto::derive_key_with(password, &salt, kdf);
    vaultkey::set(Some(key));
    if let Err(e) = crate::store::import_all_secrets(&plain) {
        return json!({ "ok": false, "error": e });
    }
    let verifier = match crypto::aes_encrypt(VERIFY_TOKEN, &key) {
        Ok(v) => v,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let cfg = json!({
        "enabled": true,
        "salt": STANDARD.encode(salt),
        "verifier": verifier,
        // Параметры записываем рядом: без них поднять стойкость в следующий раз
        // можно будет только сломав все существующие хранилища.
        "kdf": { "logN": kdf.log_n, "r": kdf.r, "p": kdf.p },
    });
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
    let Some(key) = key_from(password, salt, kdf_of(&cfg)) else {
        return json!({ "ok": false, "error": "Неверный пароль" });
    };
    if crypto::aes_decrypt(verifier, &key).ok().as_deref() != Some(VERIFY_TOKEN) {
        return json!({ "ok": false, "error": "Неверный пароль" });
    }
    vaultkey::set(Some(key));
    let plain = crate::store::export_all_secrets();
    vaultkey::set(None);
    if let Err(e) = crate::store::import_all_secrets(&plain) {
        return json!({ "ok": false, "error": e });
    }
    let _ = std::fs::remove_file(vault_path());
    json!({ "ok": true })
}
