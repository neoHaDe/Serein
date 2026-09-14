//! Docker-панель: список/действия/логи через `docker` по SSH-exec (порт docker.ts).

use serde_json::{json, Value};

/// Кавычки двойные не по вкусу, а по необходимости: одинарные `cmd.exe` не считает
/// кавычками вовсе и передаёт докеру обломки шаблона. Двойные понимают все три оболочки.
pub const LIST_CMD: &str = "docker ps -a --no-trunc --format \"{{json .}}\"";

const ACTIONS: &[&str] = &["start", "stop", "restart", "remove"];

pub fn parse_list(code: i32, stdout: &str, stderr: &str) -> Value {
    if code != 0 {
        let err = stderr.trim();
        let low = err.to_lowercase();
        // «is not recognized as an internal or external command» - это `cmd.exe`
        // сообщает, что программы нет. Текст непривычный, смысл тот же.
        let msg = if low.contains("not found")
            || low.contains("command not found")
            || low.contains("not installed")
            || low.contains("is not recognized")
        {
            "Docker не установлен на сервере".to_string()
        } else if low.contains("permission denied") || low.contains("cannot connect") {
            "Нет доступа к Docker (нужны права / запущен ли демон?)".to_string()
        } else if !err.is_empty() {
            err.to_string()
        } else {
            "docker ps завершился с ошибкой".to_string()
        };
        return json!({ "ok": false, "error": msg });
    }

    let mut containers: Vec<Value> = Vec::new();
    for line in stdout.lines() {
        let s = line.trim();
        if s.is_empty() {
            continue;
        }
        if let Ok(p) = serde_json::from_str::<Value>(s) {
            let status = p
                .get("Status")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let id = p.get("ID").and_then(|v| v.as_str()).unwrap_or("");
            let state = p
                .get("State")
                .and_then(|v| v.as_str())
                .map(|x| x.to_string())
                .unwrap_or_else(|| {
                    if status.starts_with("Up") {
                        "running".into()
                    } else {
                        "exited".into()
                    }
                });
            containers.push(json!({
                "id": id.chars().take(12).collect::<String>(),
                "name": p.get("Names").and_then(|v| v.as_str()).unwrap_or(""),
                "image": p.get("Image").and_then(|v| v.as_str()).unwrap_or(""),
                "state": state,
                "status": status,
                "ports": p.get("Ports").and_then(|v| v.as_str()).unwrap_or(""),
                "created": p.get("CreatedAt").and_then(|v| v.as_str()).unwrap_or(""),
            }));
        }
    }
    json!({ "ok": true, "containers": containers })
}

pub fn parse_stats(code: i32, stdout: &str, stderr: &str) -> Value {
    if code != 0 {
        let err = stderr.trim();
        let msg = if !err.is_empty() {
            err.to_string()
        } else {
            "docker stats завершился с ошибкой".to_string()
        };
        return json!({ "ok": false, "error": msg });
    }
    let line = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    if line.is_empty() {
        return json!({ "ok": false, "error": "Нет данных stats" });
    }
    let p: Value = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(e) => return json!({ "ok": false, "error": format!("stats JSON: {e}") }),
    };
    json!({
        "ok": true,
        "stats": {
            "cpuPct": p.get("CPUPerc").and_then(|v| v.as_str()).unwrap_or(""),
            "memUsage": p.get("MemUsage").and_then(|v| v.as_str()).unwrap_or(""),
            "memPct": p.get("MemPerc").and_then(|v| v.as_str()).unwrap_or(""),
            "netIo": p.get("NetIO").and_then(|v| v.as_str()).unwrap_or(""),
            "blockIo": p.get("BlockIO").and_then(|v| v.as_str()).unwrap_or(""),
        }
    })
}

fn safe_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
        .collect()
}

pub fn action_cmd(id: &str, action: &str) -> Option<String> {
    if !ACTIONS.contains(&action) {
        return None;
    }
    let verb = if action == "remove" { "rm -f" } else { action };
    Some(format!("docker {verb} {}", safe_id(id)))
}

/// Замер всех работающих контейнеров одним вызовом - для колонок CPU и памяти в списке.
pub const STATS_ALL_CMD: &str = "docker stats --no-stream --format \"{{json .}}\"";

/// Замеры по контейнерам. Ключ - короткий id, первые 12 знаков: так контейнер называет
/// `docker stats`, а список отдаёт полный.
pub fn parse_stats_all(code: i32, stdout: &str, stderr: &str) -> Value {
    if code != 0 {
        let err = stderr.trim();
        let msg = if err.is_empty() { "docker stats завершился с ошибкой" } else { err };
        return json!({ "ok": false, "error": msg });
    }
    let mut stats = serde_json::Map::new();
    for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(p) = serde_json::from_str::<Value>(line) else { continue };
        let id: String = p.get("ID").and_then(|v| v.as_str()).unwrap_or("").chars().take(12).collect();
        if id.is_empty() {
            continue;
        }
        stats.insert(
            id,
            json!({
                "cpuPct": p.get("CPUPerc").and_then(|v| v.as_str()).unwrap_or(""),
                "memUsage": p.get("MemUsage").and_then(|v| v.as_str()).unwrap_or(""),
                "memPct": p.get("MemPerc").and_then(|v| v.as_str()).unwrap_or(""),
            }),
        );
    }
    json!({ "ok": true, "stats": stats })
}

pub fn stats_cmd(id: &str) -> String {
    format!(
        "docker stats --no-stream --format \"{{{{json .}}}}\" {}",
        safe_id(id)
    )
}

pub fn logs_cmd(id: &str) -> String {
    format!("docker logs --tail 200 -f {} 2>&1", safe_id(id))
}

fn safe_container_path(p: &str) -> Option<String> {
    let p = p.trim();
    if p.is_empty()
        || p.contains("..")
        || p.contains('\n')
        || p.contains(';')
        || !p.starts_with('/')
    {
        return None;
    }
    let clean: String = p
        .chars()
        .filter(|c| {
            c.is_alphanumeric() || *c == '/' || *c == '.' || *c == '_' || *c == '-' || *c == ' '
        })
        .collect();
    if clean.is_empty() {
        None
    } else {
        Some(clean)
    }
}

pub fn files_cmd(id: &str, path: &str) -> Option<String> {
    let p = safe_container_path(path)?;
    let cid = safe_id(id);
    if cid.is_empty() {
        return None;
    }
    Some(format!("docker exec {} ls -1F -- {}", cid, shell_quote(&p)))
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn parse_files(code: i32, stdout: &str, stderr: &str, path: &str) -> Value {
    if code != 0 {
        let err = stderr.trim();
        let msg = if !err.is_empty() {
            err.to_string()
        } else {
            "Не удалось прочитать каталог".to_string()
        };
        return json!({ "ok": false, "error": msg, "path": path });
    }
    let mut entries: Vec<Value> = Vec::new();
    for line in stdout.lines() {
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        let (name, kind) = if raw.ends_with('/') {
            (raw.trim_end_matches('/'), "dir")
        } else if raw.ends_with('@') {
            (raw.trim_end_matches('@'), "link")
        } else if raw.ends_with('*') {
            (raw.trim_end_matches('*'), "file")
        } else {
            (raw, "file")
        };
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        entries.push(json!({ "name": name, "kind": kind }));
    }
    entries.sort_by(|a, b| {
        let ka = a["kind"].as_str().unwrap_or("");
        let kb = b["kind"].as_str().unwrap_or("");
        ka.cmp(kb).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        })
    });
    json!({ "ok": true, "path": path, "entries": entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn отсутствие_докера_на_windows_объясняется_словами() {
        // `cmd.exe` сообщает об отсутствии программы своей формулировкой, и она уходила
        // в панель как есть: «is not recognized as an internal or external command».
        let v = parse_list(
            1,
            "",
            "'docker' is not recognized as an internal or external command,
operable program or batch file.",
        );
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "Docker не установлен на сервере");
    }

    #[test]
    fn parse_list_ok() {
        let out = r#"{"ID":"abc123def456","Names":"web","Image":"nginx:latest","State":"running","Status":"Up 2 hours","Ports":"0.0.0.0:3000->3000/tcp","CreatedAt":"2024-01-01 12:00:00 +0000 UTC"}"#;
        let v = parse_list(0, out, "");
        assert!(v["ok"].as_bool().unwrap());
        let c = &v["containers"][0];
        assert_eq!(c["id"], "abc123def456");
        assert_eq!(c["name"], "web");
        assert_eq!(c["ports"], "0.0.0.0:3000->3000/tcp");
        assert_eq!(c["created"], "2024-01-01 12:00:00 +0000 UTC");
    }

    #[test]
    fn parse_stats_ok() {
        let out = r#"{"CPUPerc":"13.45%","MemUsage":"482MiB / 2GiB","MemPerc":"23.50%","NetIO":"1kB / 2kB","BlockIO":"0B / 0B"}"#;
        let v = parse_stats(0, out, "");
        assert!(v["ok"].as_bool().unwrap());
        assert_eq!(v["stats"]["cpuPct"], "13.45%");
        assert_eq!(v["stats"]["memUsage"], "482MiB / 2GiB");
    }

    #[test]
    fn замеры_всех_контейнеров_раскладываются_по_короткому_id() {
        let out = concat!(
            r#"{"ID":"abc123def456","Name":"web","CPUPerc":"13.45%","MemUsage":"482MiB / 2GiB","MemPerc":"23.50%"}"#,
            "\n\n",
            "не json\n",
            r#"{"ID":"0123456789abcdef","Name":"db","CPUPerc":"0.10%","MemUsage":"1.2GiB / 2GiB","MemPerc":"60.00%"}"#,
        );
        let v = parse_stats_all(0, out, "");
        assert_eq!(v["ok"], true);
        assert_eq!(v["stats"]["abc123def456"]["cpuPct"], "13.45%");
        assert_eq!(v["stats"]["0123456789ab"]["memUsage"], "1.2GiB / 2GiB", "ключ - первые 12 знаков");
        assert_eq!(v["stats"].as_object().unwrap().len(), 2, "мусорная строка пропущена");
        assert_eq!(parse_stats_all(0, "", "")["stats"], json!({}), "нет работающих - пусто, не ошибка");
        assert_eq!(parse_stats_all(1, "", "boom")["error"], "boom");
    }

    #[test]
    fn action_whitelist() {
        assert!(action_cmd("abc", "restart").is_some());
        assert!(action_cmd("abc", "rm").is_none());
    }
}
