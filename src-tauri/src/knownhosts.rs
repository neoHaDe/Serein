//! TOFU-хранилище отпечатков ключей серверов (порт knownhosts.ts).
//! Первый ключ запоминается; при несовпадении подключение отклоняется.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("term-tauri")
        .join("known_hosts.json")
}

fn read() -> Value {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

fn write(v: &Value) {
    if let Ok(txt) = serde_json::to_string_pretty(v) {
        let _ = std::fs::write(path(), txt);
    }
}

/// Отпечаток в формате OpenSSH: "SHA256:<base64 без паддинга>" из base64-блоба ключа.
pub fn fingerprint_from_b64(pubkey_b64: &str) -> String {
    let bytes = STANDARD.decode(pubkey_b64).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    format!("SHA256:{}", STANDARD_NO_PAD.encode(digest))
}

pub fn host_id(host: &str, port: u16) -> String {
    format!("{host}:{}", if port == 0 { 22 } else { port })
}

/// TOFU-проверка: неизвестный хост — запоминаем и принимаем; совпадение — принимаем;
/// несовпадение — отклоняем (true = доверяем и продолжаем).
pub fn check_and_remember(host_id: &str, fp: &str) -> bool {
    let mut data = read();
    match data.get(host_id).and_then(|v| v.as_str()) {
        None => {
            if let Some(o) = data.as_object_mut() {
                o.insert(host_id.to_string(), json!(fp));
            }
            write(&data);
            true
        }
        Some(known) => known == fp,
    }
}
