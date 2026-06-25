//! Импорт серверов из ~/.ssh/config и сессий PuTTY (порт importers.ts).

use crate::store;
use serde_json::{json, Value};

/// Импорт ~/.ssh/config. Возвращает число добавленных хостов.
pub fn import_ssh_config() -> Result<usize, String> {
    let path = dirs::home_dir()
        .ok_or("Домашний каталог не найден")?
        .join(".ssh")
        .join("config");
    let txt = std::fs::read_to_string(&path).map_err(|_| "Файл ~/.ssh/config не найден".to_string())?;

    let mut count = 0usize;
    let mut cur: Option<(String, Value)> = None;

    let flush = |cur: &mut Option<(String, Value)>, count: &mut usize| {
        if let Some((alias, mut srv)) = cur.take() {
            if !alias.contains('*') && !alias.contains('?') {
                if srv.get("host").is_none() {
                    srv["host"] = json!(alias);
                }
                let _ = store::servers_save(srv);
                *count += 1;
            }
        }
    };

    for raw in txt.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = match line.split_once(|c: char| c == ' ' || c == '\t' || c == '=') {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim().to_string()),
            None => continue,
        };
        if key == "host" {
            flush(&mut cur, &mut count);
            let alias = val.split_whitespace().next().unwrap_or("").to_string();
            cur = Some((
                alias.clone(),
                json!({ "id": "", "name": alias, "port": 22, "username": "root", "authType": "password", "group": "Импорт SSH" }),
            ));
        } else if let Some((_, srv)) = cur.as_mut() {
            match key.as_str() {
                "hostname" => srv["host"] = json!(val),
                "user" => srv["username"] = json!(val),
                "port" => {
                    if let Ok(p) = val.parse::<u64>() {
                        srv["port"] = json!(p);
                    }
                }
                "identityfile" => {
                    srv["privateKeyPath"] = json!(expand_tilde(&val));
                    srv["authType"] = json!("key");
                }
                "proxyjump" => srv["_proxyJumpAlias"] = json!(val),
                _ => {}
            }
        }
    }
    flush(&mut cur, &mut count);
    Ok(count)
}

fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    p.to_string()
}

/// Импорт сессий PuTTY из реестра HKCU\Software\SimonTatham\PuTTY\Sessions.
#[cfg(windows)]
pub fn import_putty() -> Result<usize, String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let sessions = hkcu
        .open_subkey("Software\\SimonTatham\\PuTTY\\Sessions")
        .map_err(|_| "Сессии PuTTY не найдены".to_string())?;

    let mut count = 0usize;
    for name in sessions.enum_keys().flatten() {
        let sk = match sessions.open_subkey(&name) {
            Ok(k) => k,
            Err(_) => continue,
        };
        let host: String = sk.get_value("HostName").unwrap_or_default();
        if host.is_empty() {
            continue;
        }
        let port: u32 = sk.get_value("PortNumber").unwrap_or(22);
        let user: String = sk.get_value("UserName").unwrap_or_else(|_| "root".into());
        let display = urldecode(&name);
        let _ = store::servers_save(json!({
            "id": "", "name": display, "host": host, "port": port,
            "username": if user.is_empty() { "root".into() } else { user },
            "authType": "password", "group": "Импорт PuTTY"
        }));
        count += 1;
    }
    Ok(count)
}

#[cfg(not(windows))]
pub fn import_putty() -> Result<usize, String> {
    Err("Импорт PuTTY доступен только на Windows".into())
}

#[cfg(windows)]
fn urldecode(s: &str) -> String {
    // PuTTY кодирует имена сессий %XX.
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}
