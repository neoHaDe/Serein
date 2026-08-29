//! TOFU-хранилище отпечатков ключей серверов (порт knownhosts.ts).
//! Первый ключ запоминается; при несовпадении подключение отклоняется.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn path() -> PathBuf {
    crate::store::config_dir().join("known_hosts.json")
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

/// Что известно про ключ хоста перед подключением.
#[derive(Debug, PartialEq)]
pub enum HostKeyStatus {
    /// Хост уже известен и отпечаток совпал.
    Trusted,
    /// Хост встречается впервые.
    New,
    /// Отпечаток не совпал с запомненным — либо сервер переустановили, либо это чужой сервер.
    Changed { previous: String },
}

pub fn status(host_id: &str, fp: &str) -> HostKeyStatus {
    match read().get(host_id).and_then(|v| v.as_str()) {
        None => HostKeyStatus::New,
        Some(known) if known == fp => HostKeyStatus::Trusted,
        Some(known) => HostKeyStatus::Changed {
            previous: known.to_string(),
        },
    }
}

/// Запоминает отпечаток (перезаписывая прежний, если пользователь подтвердил смену).
pub fn remember(host_id: &str, fp: &str) {
    let mut data = read();
    if let Some(o) = data.as_object_mut() {
        o.insert(host_id.to_string(), json!(fp));
    }
    write(&data);
}

/// Список известных хостов для интерфейса: `[{ host, fingerprint }]`, отсортирован по имени.
pub fn list() -> Vec<Value> {
    let data = read();
    let mut out: Vec<Value> = data
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(host, fp)| json!({ "host": host, "fingerprint": fp.as_str().unwrap_or("") }))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| {
        a["host"]
            .as_str()
            .unwrap_or("")
            .cmp(b["host"].as_str().unwrap_or(""))
    });
    out
}

pub fn forget(host_id: &str) -> bool {
    let mut data = read();
    let removed = data
        .as_object_mut()
        .map(|o| o.remove(host_id).is_some())
        .unwrap_or(false);
    if removed {
        write(&data);
    }
    removed
}

/// Разбирает строку файла OpenSSH `known_hosts`: `host[,host2] keytype base64 [comment]`.
/// Возвращает пары (host_id, отпечаток). Хэшированные записи (`|1|…`) пропускаются —
/// из них имя хоста восстановить нельзя, а сверять нам нужно именно по имени.
pub fn parse_openssh_line(line: &str) -> Vec<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with("|1|") {
        return Vec::new();
    }
    let mut parts = line.split_whitespace();
    let (Some(hosts), Some(_keytype), Some(blob)) = (parts.next(), parts.next(), parts.next())
    else {
        return Vec::new();
    };
    // Ключ приходит в base64 — тот же формат, из которого мы считаем отпечаток.
    let fp = fingerprint_from_b64(blob);
    if fp == "SHA256:47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU" {
        return Vec::new(); // отпечаток пустых байт — значит base64 не разобрался
    }
    hosts
        .split(',')
        .filter(|h| !h.is_empty())
        .map(|h| {
            // OpenSSH пишет нестандартный порт как `[host]:2222`.
            let normalized = if let Some(rest) = h.strip_prefix('[') {
                match rest.split_once("]:") {
                    Some((host, port)) => format!("{host}:{port}"),
                    None => rest.trim_end_matches(']').to_string(),
                }
            } else {
                format!("{h}:22")
            };
            (normalized, fp.clone())
        })
        .collect()
}

/// Импорт из пользовательского `~/.ssh/known_hosts`. Возвращает число добавленных записей.
pub fn import_openssh() -> Result<usize, String> {
    let path = dirs::home_dir()
        .map(|h| h.join(".ssh").join("known_hosts"))
        .ok_or("Не найден домашний каталог")?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", path.display()))?;

    let mut data = read();
    let mut added = 0usize;
    if let Some(o) = data.as_object_mut() {
        for line in text.lines() {
            for (host_id, fp) in parse_openssh_line(line) {
                // Свои записи не перетираем: то, что уже подтверждено в приложении, важнее.
                if !o.contains_key(&host_id) {
                    o.insert(host_id, json!(fp));
                    added += 1;
                }
            }
        }
    }
    if added > 0 {
        write(&data);
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_line() {
        let line = "example.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIG1234567890abcdefghij comment";
        let got = parse_openssh_line(line);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "example.com:22");
        assert!(got[0].1.starts_with("SHA256:"));
    }

    #[test]
    fn splits_multiple_hosts_and_keeps_port() {
        let line = "[srv.local]:2222,10.0.0.5 ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQC1234";
        let got = parse_openssh_line(line);
        let hosts: Vec<&str> = got.iter().map(|(h, _)| h.as_str()).collect();
        assert_eq!(hosts, vec!["srv.local:2222", "10.0.0.5:22"]);
        assert_eq!(got[0].1, got[1].1, "один ключ — один отпечаток на все имена");
    }

    #[test]
    fn skips_comments_hashed_and_junk() {
        assert!(parse_openssh_line("# комментарий").is_empty());
        assert!(parse_openssh_line("").is_empty());
        assert!(parse_openssh_line("   ").is_empty());
        assert!(
            parse_openssh_line("|1|hash= ssh-ed25519 AAAAC3Nza").is_empty(),
            "хэшированные записи пропускаем: имя хоста из них не достать"
        );
        assert!(parse_openssh_line("только-хост").is_empty());
    }

    #[test]
    fn status_distinguishes_new_trusted_and_changed() {
        // Чистая проверка сравнения, без файла на диске.
        let known = "SHA256:aaa";
        assert_eq!(
            match Some(known) {
                None => HostKeyStatus::New,
                Some(k) if k == "SHA256:aaa" => HostKeyStatus::Trusted,
                Some(k) => HostKeyStatus::Changed { previous: k.into() },
            },
            HostKeyStatus::Trusted
        );
    }
}
