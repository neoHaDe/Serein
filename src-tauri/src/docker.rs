//! Docker-панель: список/действия/логи через `docker` по SSH-exec (порт docker.ts).

use serde_json::{json, Value};

pub const LIST_CMD: &str = "docker ps -a --no-trunc --format '{{json .}}'";

pub fn parse_list(code: i32, stdout: &str, stderr: &str) -> Value {
    if code != 0 {
        let err = stderr.trim();
        let low = err.to_lowercase();
        let msg = if low.contains("not found") || low.contains("command not found") || low.contains("not installed") {
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
                .unwrap_or_else(|| if status.starts_with("Up") { "running".into() } else { "exited".into() });
            containers.push(json!({
                "id": id.chars().take(12).collect::<String>(),
                "name": p.get("Names").and_then(|v| v.as_str()).unwrap_or(""),
                "image": p.get("Image").and_then(|v| v.as_str()).unwrap_or(""),
                "state": state,
                "status": status,
            }));
        }
    }
    json!({ "ok": true, "containers": containers })
}

fn safe_id(id: &str) -> String {
    id.chars().filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_').collect()
}

pub fn action_cmd(id: &str, action: &str) -> String {
    let verb = if action == "remove" { "rm -f" } else { action };
    format!("docker {verb} {}", safe_id(id))
}

pub fn logs_cmd(id: &str) -> String {
    format!("docker logs --tail 300 {} 2>&1", safe_id(id))
}
