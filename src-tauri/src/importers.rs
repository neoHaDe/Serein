//! Импорт серверов из ~/.ssh/config, PuTTY, MobaXterm, XShell и SecureCRT.

use crate::store;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Итог импорта ~/.ssh/config.
pub struct SshConfigImport {
    /// Сколько серверов добавлено.
    pub imported: usize,
    /// Серверы, чей ProxyJump не удалось сопоставить со шлюзом из списка: они добавлены
    /// без прыжка, и это надо сказать человеку, а не оставить молча прямым подключением.
    pub unresolved_jumps: Vec<String>,
}

/// Импорт ~/.ssh/config.
pub fn import_ssh_config() -> Result<SshConfigImport, String> {
    let path = dirs::home_dir()
        .ok_or("Домашний каталог не найден")?
        .join(".ssh")
        .join("config");
    let txt = std::fs::read_to_string(&path).map_err(|_| "Файл ~/.ssh/config не найден".to_string())?;
    let plan = plan_ssh_config(&txt, &store::servers_list());
    let mut imported = 0usize;
    for srv in plan.servers {
        store::servers_save(srv)?;
        imported += 1;
    }
    Ok(SshConfigImport {
        imported,
        unresolved_jumps: plan.unresolved_jumps,
    })
}

struct SshConfigPlan {
    servers: Vec<Value>,
    unresolved_jumps: Vec<String>,
}

/// Что добавить из ~/.ssh/config, не трогая диск.
///
/// Три вещи, которые раньше терялись. ProxyJump разбирался и выбрасывался: сервер за
/// бастионом приезжал прямым подключением. Повторный импорт дублировал весь список. И
/// строки после `Match` дописывались к предыдущему `Host`.
///
/// Шлюз ищется среди хостов этого же файла и среди уже известных серверов (по имени, затем
/// по адресу). Цепочку из нескольких прыжков (`a,b`) одним полем `proxyJump` не выразить -
/// такой сервер добавляется без прыжка и попадает в `unresolved_jumps`.
fn plan_ssh_config(txt: &str, existing: &[Value]) -> SshConfigPlan {
    let mut hosts: Vec<(String, Value, Option<String>)> = Vec::new();
    let mut cur: Option<(String, Value, Option<String>)> = None;
    for raw in txt.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = match line.split_once([' ', '\t', '=']) {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim().to_string()),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                hosts.extend(cur.take());
                let alias = val.split_whitespace().next().unwrap_or("").to_string();
                let srv = json!({ "name": alias, "port": 22, "username": "root", "authType": "password", "group": "Импорт SSH" });
                cur = Some((alias, srv, None));
            }
            // Блок `Match` - условные настройки, а не новый хост. Всё до следующего `Host`
            // к предыдущему хосту не относится.
            "match" => hosts.extend(cur.take()),
            _ => {
                let Some((_, srv, jump)) = cur.as_mut() else { continue };
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
                    // Как и в OpenSSH, действует первое значение.
                    "proxyjump" if jump.is_none() => *jump = Some(val),
                    _ => {}
                }
            }
        }
    }
    hosts.extend(cur);

    let key_of = |s: &Value| -> (String, u64, String) {
        (
            s["host"].as_str().unwrap_or("").to_lowercase(),
            s["port"].as_u64().unwrap_or(22),
            s["username"].as_str().unwrap_or("").to_string(),
        )
    };
    let id_of = |s: &Value| s["id"].as_str().unwrap_or("").to_string();

    // Имя хоста в файле -> номер сервера: нового или уже известного с тем же адресом.
    let mut alias_ids: Vec<(String, String)> = Vec::new();
    let mut fresh: Vec<(Value, Option<String>)> = Vec::new();
    for (alias, mut srv, jump) in hosts {
        if alias.is_empty() || alias.contains('*') || alias.contains('?') {
            continue;
        }
        if srv.get("host").is_none() {
            srv["host"] = json!(alias);
        }
        if let Some(known) = existing.iter().find(|e| key_of(e) == key_of(&srv)) {
            alias_ids.push((alias, id_of(known)));
            continue;
        }
        let id = uuid::Uuid::new_v4().to_string();
        srv["id"] = json!(id);
        alias_ids.push((alias, id));
        fresh.push((srv, jump));
    }

    let resolve = |hop: &str| -> Option<String> {
        // `user@host:port` -> `host`: в карточке шлюза свой пользователь и порт.
        let name = hop.rsplit('@').next().unwrap_or(hop);
        let name = name.split(':').next().unwrap_or(name);
        alias_ids
            .iter()
            .find(|(a, _)| a == name)
            .map(|(_, id)| id.clone())
            .or_else(|| existing.iter().find(|e| e["name"] == name).map(&id_of))
            .or_else(|| {
                existing
                    .iter()
                    .find(|e| e["host"].as_str().is_some_and(|h| h.eq_ignore_ascii_case(name)))
                    .map(&id_of)
            })
    };

    let mut unresolved_jumps = Vec::new();
    let mut servers = Vec::new();
    for (mut srv, jump) in fresh {
        if let Some(hop) = jump.filter(|j| !j.eq_ignore_ascii_case("none")) {
            match (!hop.contains(',')).then(|| resolve(hop.trim())).flatten() {
                Some(id) => srv["proxyJump"] = json!(id),
                None => unresolved_jumps.push(srv["name"].as_str().unwrap_or("").to_string()),
            }
        }
        servers.push(srv);
    }
    SshConfigPlan {
        servers,
        unresolved_jumps,
    }
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

// Нужно только на Windows: на других системах не вызывается, и это не долг, а платформа.
#[cfg_attr(not(windows), allow(dead_code))]
fn save_imported_server(
    name: &str,
    host: &str,
    port: u16,
    username: &str,
    group: &str,
    auth_type: &str,
    private_key_path: Option<&str>,
) -> Result<(), String> {
    let mut srv = json!({
        "id": "",
        "name": name,
        "host": host,
        "port": port,
        "username": username,
        "authType": auth_type,
        "group": group,
    });
    if let Some(p) = private_key_path.filter(|s| !s.is_empty()) {
        srv["privateKeyPath"] = json!(p);
    }
    store::servers_save(srv).map(|_| ())
}

// Нужно только на Windows: на других системах не вызывается, и это не долг, а платформа.
#[cfg_attr(not(windows), allow(dead_code))]
/// MobaXterm: закладка SSH - `#109#0%host%port%user%...`
pub fn parse_mobaxterm_ssh_bookmark(name: &str, val: &str, _group: &str) -> Option<(String, String, u16, String)> {
    if !val.contains("#109#") {
        return None;
    }
    let parts: Vec<&str> = val.split('%').collect();
    if parts.len() < 4 {
        return None;
    }
    let host = parts[1].trim();
    if host.is_empty() {
        return None;
    }
    let port = parts[2].trim().parse::<u16>().unwrap_or(22);
    let user = parts[3].trim();
    let username = if user.is_empty() { "root" } else { user }.to_string();
    let display = if name.trim().is_empty() {
        host.to_string()
    } else {
        name.trim().to_string()
    };
    Some((display, host.to_string(), port, username))
}

// Нужно только на Windows: на других системах не вызывается, и это не долг, а платформа.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn import_mobaxterm_text(txt: &str, default_group: &str) -> usize {
    let mut group = default_group.to_string();
    let mut count = 0usize;
    for raw in txt.lines() {
        let line = raw.trim();
        if line.starts_with("SubRep=") {
            group = line.strip_prefix("SubRep=").unwrap_or(default_group).trim().to_string();
            if group.is_empty() {
                group = default_group.to_string();
            }
            continue;
        }
        let Some((name, val)) = line.split_once('=') else {
            continue;
        };
        let Some((display, host, port, username)) = parse_mobaxterm_ssh_bookmark(name.trim(), val.trim(), &group)
        else {
            continue;
        };
        if save_imported_server(&display, &host, port, &username, &group, "password", None).is_ok() {
            count += 1;
        }
    }
    count
}

#[cfg(windows)]
pub fn import_mobaxterm() -> Result<usize, String> {
    let mut total = 0usize;
    let mut found_any = false;
    for path in mobaxterm_config_paths() {
        if !path.is_file() {
            continue;
        }
        found_any = true;
        let txt = std::fs::read_to_string(&path).unwrap_or_default();
        total += import_mobaxterm_text(&txt, "Импорт MobaXterm");
    }
    if !found_any {
        return Err("Файлы MobaXterm (MobaXterm.ini) не найдены".into());
    }
    Ok(total)
}

#[cfg(not(windows))]
pub fn import_mobaxterm() -> Result<usize, String> {
    Err("Импорт MobaXterm доступен только на Windows".into())
}

#[cfg(windows)]
fn mobaxterm_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(data) = dirs::data_dir() {
        paths.push(data.join("MobaXterm").join("MobaXterm.ini"));
    }
    if let Some(doc) = dirs::document_dir() {
        let moba = doc.join("MobaXterm");
        paths.push(moba.join("MobaXterm.ini"));
        if moba.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&moba) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("mxtsessions") {
                        paths.push(p);
                    }
                }
            }
        }
    }
    paths
}

// Нужно только на Windows: на других системах не вызывается, и это не долг, а платформа.
#[cfg_attr(not(windows), allow(dead_code))]
/// XShell `.xsh`: секции `[CONNECTION]` и `[CONNECTION:AUTHENTICATION]`.
pub fn parse_xshell_session(
    content: &str,
    fallback_name: &str,
) -> Option<(String, String, u16, String, Option<String>)> {
    let mut section = String::new();
    let mut host = None;
    let mut port = 22u16;
    let mut protocol = None;
    let mut username = None;
    let mut auth_method = None;
    let mut user_key = None;
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line.to_ascii_uppercase();
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();
        match key {
            "Host" if host.is_none() || section.contains("CONNECTION") => host = Some(val.to_string()),
            "Port" if section.contains("CONNECTION") => port = val.parse().unwrap_or(22),
            "Protocol" if section.contains("CONNECTION") => protocol = Some(val.to_ascii_uppercase()),
            "Method" if section.contains("AUTHENTICATION") => auth_method = Some(val.to_string()),
            "UserName" if section.contains("AUTHENTICATION") || username.is_none() => username = Some(val.to_string()),
            "UserKey" | "PublicKeyPath" if user_key.is_none() => user_key = Some(val.to_string()),
            _ => {}
        }
    }
    if protocol.as_deref() != Some("SSH") {
        return None;
    }
    let host = host?.trim().to_string();
    if host.is_empty() {
        return None;
    }
    let user = username.filter(|u| !u.is_empty()).unwrap_or_else(|| "root".into());
    let name = if fallback_name.is_empty() {
        host.clone()
    } else {
        fallback_name.to_string()
    };
    let key_path = if auth_method.as_deref() == Some("PublicKey") {
        user_key
    } else {
        None
    };
    Some((name, host, port, user, key_path))
}

// Нужно только на Windows: на других системах не вызывается, и это не долг, а платформа.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn import_xshell_file(content: &str, fallback_name: &str, group: &str) -> bool {
    let Some((name, host, port, username, key_path)) = parse_xshell_session(content, fallback_name) else {
        return false;
    };
    let auth = if key_path.is_some() { "key" } else { "password" };
    save_imported_server(&name, &host, port, &username, group, auth, key_path.as_deref()).is_ok()
}

#[cfg(windows)]
pub fn import_xshell() -> Result<usize, String> {
    let roots = xshell_session_roots();
    if roots.is_empty() {
        return Err("Каталоги сессий XShell не найдены".into());
    }
    let mut files = Vec::new();
    for root in &roots {
        collect_files_with_ext(root, "xsh", &mut files);
    }
    if files.is_empty() {
        return Err("Файлы сессий XShell (*.xsh) не найдены".into());
    }
    let mut count = 0usize;
    for path in files {
        let txt = std::fs::read_to_string(&path).unwrap_or_default();
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if import_xshell_file(&txt, name, "Импорт XShell") {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(not(windows))]
pub fn import_xshell() -> Result<usize, String> {
    Err("Импорт XShell доступен только на Windows".into())
}

#[cfg(windows)]
fn xshell_session_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(doc) = dirs::document_dir() {
        let ns = doc.join("NetSarang Computer");
        if ns.is_dir() {
            if let Ok(versions) = std::fs::read_dir(&ns) {
                for ver in versions.flatten() {
                    let sessions = ver.path().join("Xshell").join("Sessions");
                    if sessions.is_dir() {
                        roots.push(sessions);
                    }
                }
            }
        }
    }
    if let Some(data) = dirs::data_dir() {
        let sessions = data.join("NetSarang").join("Xshell").join("Sessions");
        if sessions.is_dir() {
            roots.push(sessions);
        }
    }
    roots
}

// Нужно только на Windows: на других системах не вызывается, и это не долг, а платформа.
#[cfg_attr(not(windows), allow(dead_code))]
/// SecureCRT: построчно `H:`, `P:`, `U:`, `S:`, `D:` (SSH2).
pub fn parse_securecrt_session(content: &str, fallback_name: &str) -> Option<(String, String, u16, String)> {
    let mut name = fallback_name.to_string();
    let mut host = None;
    let mut port = 22u16;
    let mut user = None;
    let mut proto_ssh = false;
    let mut saw_d = false;
    for raw in content.lines() {
        let line = raw.trim();
        if line.len() < 3 || line.as_bytes()[1] != b':' {
            continue;
        }
        let key = line.as_bytes()[0] as char;
        let val = line[2..].trim().trim_matches('"');
        match key {
            'S' => name = val.to_string(),
            'H' => host = Some(val.to_string()),
            'P' => port = val.parse().unwrap_or(22),
            'U' => user = Some(val.to_string()),
            'D' => {
                saw_d = true;
                let lower = val.to_ascii_lowercase();
                if lower.contains("ssh") {
                    proto_ssh = true;
                }
            }
            _ => {}
        }
    }
    if !saw_d {
        proto_ssh = true;
    }
    if !proto_ssh {
        return None;
    }
    let host = host?.trim().to_string();
    if host.is_empty() {
        return None;
    }
    let username = user.filter(|u| !u.is_empty()).unwrap_or_else(|| "root".into());
    let display = if name.trim().is_empty() { host.clone() } else { name };
    Some((display, host, port, username))
}

// Нужно только на Windows: на других системах не вызывается, и это не долг, а платформа.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn import_securecrt_file(content: &str, fallback_name: &str, group: &str) -> bool {
    let Some((name, host, port, username)) = parse_securecrt_session(content, fallback_name) else {
        return false;
    };
    save_imported_server(&name, &host, port, &username, group, "password", None).is_ok()
}

#[cfg(windows)]
pub fn import_securecrt() -> Result<usize, String> {
    let Some(root) = securecrt_sessions_dir() else {
        return Err("Каталог сессий SecureCRT не найден".into());
    };
    if !root.is_dir() {
        return Err("Каталог сессий SecureCRT не найден".into());
    }
    let mut files = Vec::new();
    collect_files_with_ext(&root, "ini", &mut files);
    if files.is_empty() {
        return Err("Файлы сессий SecureCRT (*.ini) не найдены".into());
    }
    let mut count = 0usize;
    for path in files {
        let txt = std::fs::read_to_string(&path).unwrap_or_default();
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if import_securecrt_file(&txt, name, "Импорт SecureCRT") {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(not(windows))]
pub fn import_securecrt() -> Result<usize, String> {
    Err("Импорт SecureCRT доступен только на Windows".into())
}

#[cfg(windows)]
fn securecrt_sessions_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|c| c.join("VanDyke").join("Config").join("Sessions"))
}

// Нужно только на Windows: на других системах не вызывается, и это не долг, а платформа.
#[cfg_attr(not(windows), allow(dead_code))]
fn collect_files_with_ext(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_ext(&path, ext, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobaxterm_ssh_bookmark() {
        let val = "#109#0%192.168.0.10%2222%admin%%-1%";
        let parsed = parse_mobaxterm_ssh_bookmark("srv1", val, "G").unwrap();
        assert_eq!(parsed.0, "srv1");
        assert_eq!(parsed.1, "192.168.0.10");
        assert_eq!(parsed.2, 2222);
        assert_eq!(parsed.3, "admin");
    }

    #[test]
    fn mobaxterm_skips_non_ssh() {
        assert!(parse_mobaxterm_ssh_bookmark("t", "#110#0%h%22%u%", "G").is_none());
    }

    #[test]
    fn xshell_ssh_session() {
        let txt = r#"
[CONNECTION]
Host=host.example.com
Port=2200
Protocol=SSH
[CONNECTION:AUTHENTICATION]
UserName=alice
Method=PublicKey
UserKey=C:\Users\me\.ssh\id_rsa
"#;
        let p = parse_xshell_session(txt, "fallback").unwrap();
        assert_eq!(p.0, "fallback");
        assert_eq!(p.1, "host.example.com");
        assert_eq!(p.2, 2200);
        assert_eq!(p.3, "alice");
        assert_eq!(p.4.as_deref(), Some(r"C:\Users\me\.ssh\id_rsa"));
    }

    #[test]
    fn xshell_skips_non_ssh() {
        let txt = "[CONNECTION]\nHost=h\nPort=23\nProtocol=TELNET\n";
        assert!(parse_xshell_session(txt, "t").is_none());
    }

    #[test]
    fn securecrt_ssh_session() {
        let txt = r#"
S:"My Session"
D:0 (SSH2)
H:192.168.1.2
P:22
U:root
"#;
        let p = parse_securecrt_session(txt, "file").unwrap();
        assert_eq!(p.0, "My Session");
        assert_eq!(p.1, "192.168.1.2");
        assert_eq!(p.2, 22);
        assert_eq!(p.3, "root");
    }

    #[test]
    fn securecrt_skips_telnet() {
        let txt = "D:Telnet\nH:h\nU:u\n";
        assert!(parse_securecrt_session(txt, "x").is_none());
    }

    const SSH_CONFIG: &str = "
Host *
    ServerAliveInterval 30

Host bastion
    HostName 10.0.0.1
    User jump

Host web-02
    HostName 10.0.1.2
    User deploy
    ProxyJump bastion
    IdentityFile ~/.ssh/id_ed25519
";

    fn by_name<'a>(servers: &'a [Value], name: &str) -> &'a Value {
        servers.iter().find(|s| s["name"] == name).expect(name)
    }

    #[test]
    fn ssh_config_proxy_jump_points_at_the_bastion() {
        let plan = plan_ssh_config(SSH_CONFIG, &[]);
        assert_eq!(plan.servers.len(), 2, "шаблон `Host *` не сервер");
        let bastion = by_name(&plan.servers, "bastion");
        let web = by_name(&plan.servers, "web-02");
        assert_eq!(web["proxyJump"], bastion["id"]);
        assert!(bastion.get("proxyJump").is_none());
        assert_eq!(web["authType"], "key");
        assert!(plan.unresolved_jumps.is_empty());
        assert!(
            web.get("_proxyJumpAlias").is_none(),
            "служебное поле не должно попадать в профиль"
        );
    }

    #[test]
    fn ssh_config_jump_to_an_existing_server() {
        let existing =
            [json!({ "id": "known-bastion", "name": "bastion", "host": "10.0.0.1", "port": 22, "username": "jump" })];
        let plan = plan_ssh_config(SSH_CONFIG, &existing);
        assert_eq!(plan.servers.len(), 1, "шлюз уже в списке - второй раз не добавляется");
        assert_eq!(plan.servers[0]["name"], "web-02");
        assert_eq!(plan.servers[0]["proxyJump"], "known-bastion");
    }

    #[test]
    fn ssh_config_second_import_adds_nothing() {
        let first = plan_ssh_config(SSH_CONFIG, &[]);
        let again = plan_ssh_config(SSH_CONFIG, &first.servers);
        assert!(again.servers.is_empty());
    }

    #[test]
    fn ssh_config_unresolved_jump_is_reported() {
        let txt =
            "Host a\n  HostName 10.0.0.5\n  ProxyJump one,two\nHost b\n  HostName 10.0.0.6\n  ProxyJump nowhere\n";
        let plan = plan_ssh_config(txt, &[]);
        assert_eq!(plan.servers.len(), 2);
        assert!(plan.servers.iter().all(|s| s.get("proxyJump").is_none()));
        assert_eq!(plan.unresolved_jumps, vec!["a", "b"]);
    }

    #[test]
    fn ssh_config_jump_with_user_and_port_finds_the_host() {
        let txt = "Host gw\n  HostName 10.0.0.1\nHost db\n  HostName 10.0.2.2\n  ProxyJump admin@gw:2222\n";
        let plan = plan_ssh_config(txt, &[]);
        assert_eq!(
            by_name(&plan.servers, "db")["proxyJump"],
            by_name(&plan.servers, "gw")["id"]
        );
    }

    #[test]
    fn ssh_config_match_block_does_not_leak_into_previous_host() {
        let txt = "Host web\n  HostName 10.0.0.9\n  User deploy\nMatch host *.corp\n  User root\n";
        let plan = plan_ssh_config(txt, &[]);
        assert_eq!(plan.servers.len(), 1);
        assert_eq!(plan.servers[0]["username"], "deploy");
    }
}
