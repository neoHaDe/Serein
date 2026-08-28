//! Снимки процессов / systemd / логов хоста через SSH exec.

use serde_json::{json, Value};

pub const PS_CMD: &str = "ps -eo pid=,user=,pcpu=,pmem=,stat=,comm= --sort=-pcpu 2>/dev/null | head -n 80";
pub const SERVICES_CMD: &str =
    "systemctl list-units --type=service --all --no-legend --no-pager --plain 2>/dev/null | head -n 120";
pub const LOGS_CMD: &str = concat!(
    "if command -v journalctl >/dev/null 2>&1; then journalctl -n 200 --no-pager -o short-iso 2>/dev/null; ",
    "elif [ -r /var/log/syslog ]; then tail -n 200 /var/log/syslog; ",
    "elif [ -r /var/log/messages ]; then tail -n 200 /var/log/messages; ",
    "else echo \"Нет journalctl и syslog\"; fi"
);

pub fn parse_ps(stdout: &str) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            continue;
        }
        let pid: u32 = match parts[0].parse() {
            Ok(n) if n > 0 => n,
            _ => continue,
        };
        rows.push(json!({
            "pid": pid,
            "user": parts[1],
            "cpu": parts[2].parse::<f64>().unwrap_or(0.0),
            "mem": parts[3].parse::<f64>().unwrap_or(0.0),
            "stat": parts[4],
            "cmd": parts[5..].join(" "),
        }));
    }
    json!({ "ok": true, "rows": rows })
}

pub fn parse_services(code: i32, stdout: &str, stderr: &str) -> Value {
    if code != 0 && stdout.trim().is_empty() {
        let err = stderr.trim();
        let msg = if err.to_lowercase().contains("not found") || err.is_empty() {
            "systemctl недоступен на этом хосте"
        } else {
            err
        };
        return json!({ "ok": false, "error": msg });
    }
    let mut rows: Vec<Value> = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let unit = parts[0];
        if !unit.ends_with(".service") {
            continue;
        }
        let name = unit.trim_end_matches(".service");
        rows.push(json!({
            "name": name,
            "unit": unit,
            "load": parts[1],
            "active": parts[2],
            "sub": parts[3],
            "desc": if parts.len() > 4 { parts[4..].join(" ") } else { String::new() },
        }));
    }
    json!({ "ok": true, "rows": rows })
}

pub fn kill_cmd(pid: u32) -> Result<String, String> {
    if pid <= 1 {
        return Err("Нельзя слать kill pid <= 1".into());
    }
    Ok(format!("kill {pid}"))
}

pub fn service_cmd(name: &str, action: &str) -> Result<String, String> {
    let ok = matches!(action, "start" | "stop" | "restart");
    if !ok {
        return Err("Допустимы start / stop / restart".into());
    }
    if name.is_empty()
        || name.len() > 128
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | ':'))
    {
        return Err("Некорректное имя сервиса".into());
    }
    Ok(format!("systemctl {action} -- {name}.service"))
}