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
            let status = p.get("Status").and_then(|v| v.as_str()).unwrap_or("").to_string();
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

/// Имя или id контейнера в том виде, в каком их принимает docker: латиница, цифры и `_ . -`,
/// первым знаком - буква или цифра.
///
/// Остальное - отказ, а не обрезка. Обрезанное имя указывает на другой контейнер, пустое
/// превращает `docker stats` в замер всех контейнеров сразу, а ведущий `-` docker прочёл бы
/// как ключ.
pub(crate) fn container_ref(id: &str) -> Result<&str, String> {
    let mut chars = id.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    if first_ok && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) {
        Ok(id)
    } else {
        Err(format!(
            "Недопустимое имя контейнера «{}»: только латиница, цифры и . _ -, первым - буква или цифра",
            id.escape_debug()
        ))
    }
}

pub fn action_cmd(id: &str, action: &str) -> Result<String, String> {
    if !ACTIONS.contains(&action) {
        return Err(format!("Неизвестное действие: {action}"));
    }
    let verb = if action == "remove" { "rm -f" } else { action };
    Ok(format!("docker {verb} {}", container_ref(id)?))
}

/// Замер всех работающих контейнеров одним вызовом - для колонок CPU и памяти в списке.
pub const STATS_ALL_CMD: &str = "docker stats --no-stream --format \"{{json .}}\"";

/// Замеры по контейнерам. Ключ - короткий id, первые 12 знаков: так контейнер называет
/// `docker stats`, а список отдаёт полный.
pub fn parse_stats_all(code: i32, stdout: &str, stderr: &str) -> Value {
    if code != 0 {
        let err = stderr.trim();
        let msg = if err.is_empty() {
            "docker stats завершился с ошибкой"
        } else {
            err
        };
        return json!({ "ok": false, "error": msg });
    }
    let mut stats = serde_json::Map::new();
    for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(p) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let id: String = p
            .get("ID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(12)
            .collect();
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

pub fn stats_cmd(id: &str) -> Result<String, String> {
    Ok(format!(
        "docker stats --no-stream --format \"{{{{json .}}}}\" {}",
        container_ref(id)?
    ))
}

pub fn logs_cmd(id: &str) -> Result<String, String> {
    Ok(format!("docker logs --tail 200 -f {} 2>&1", container_ref(id)?))
}

/// Путь внутри контейнера. В команду он идёт в кавычках и после `--`, поэтому годится любой
/// абсолютный путь без управляющих знаков - и с пробелами, и с кириллицей. Обрезать его нельзя:
/// откроется другой каталог.
fn container_path(p: &str) -> Result<&str, String> {
    if p.starts_with('/') && !p.chars().any(char::is_control) {
        Ok(p)
    } else {
        Err(format!("Недопустимый путь в контейнере: «{}»", p.escape_debug()))
    }
}

pub fn files_cmd(id: &str, path: &str) -> Result<String, String> {
    let cid = container_ref(id)?;
    let p = container_path(path)?;
    Ok(format!("docker exec {} ls -1F -- {}", cid, shell_quote(p)))
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
        ka.cmp(kb)
            .then_with(|| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")))
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
        assert_eq!(
            v["stats"]["0123456789ab"]["memUsage"], "1.2GiB / 2GiB",
            "ключ - первые 12 знаков"
        );
        assert_eq!(v["stats"].as_object().unwrap().len(), 2, "мусорная строка пропущена");
        assert_eq!(
            parse_stats_all(0, "", "")["stats"],
            json!({}),
            "нет работающих - пусто, не ошибка"
        );
        assert_eq!(parse_stats_all(1, "", "boom")["error"], "boom");
    }

    #[test]
    fn action_whitelist() {
        assert_eq!(action_cmd("abc", "restart").unwrap(), "docker restart abc");
        assert_eq!(action_cmd("abc", "remove").unwrap(), "docker rm -f abc");
        assert!(action_cmd("abc", "rm").is_err());
    }

    #[test]
    fn имя_контейнера_проверяется_а_не_обрезается() {
        assert_eq!(action_cmd("my_app.web-1", "stop").unwrap(), "docker stop my_app.web-1");
        // Раньше отсюда получалось `docker stop webrm-rf` - то есть другой контейнер.
        for bad in ["web;rm -rf /", "web x", "", "-f", "--help", "веб", "a\nb", "a$(id)"] {
            assert!(action_cmd(bad, "stop").is_err(), "{bad:?}");
            assert!(logs_cmd(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn пустое_имя_не_превращает_замер_в_замер_всех_контейнеров() {
        assert!(stats_cmd("").is_err());
        assert!(stats_cmd("abc").unwrap().ends_with(" abc"));
    }

    #[test]
    fn путь_в_контейнере_берётся_целиком() {
        assert_eq!(
            files_cmd("abc", "/srv/my app/данные").unwrap(),
            "docker exec abc ls -1F -- '/srv/my app/данные'"
        );
        assert_eq!(
            files_cmd("abc", "/it's").unwrap(),
            "docker exec abc ls -1F -- '/it'\\''s'"
        );
        for bad in ["relative", "", "/a\nb", "/a\0b"] {
            assert!(files_cmd("abc", bad).is_err(), "{bad:?}");
        }
        assert!(files_cmd("-x", "/").is_err());
    }
}
