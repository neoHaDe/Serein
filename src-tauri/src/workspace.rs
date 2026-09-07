//! Снимки процессов / systemd / логов хоста через SSH exec.

use serde_json::{json, Value};

pub const PS_CMD: &str = "ps -eo pid=,user=,pcpu=,pmem=,stat=,comm= --sort=-pcpu 2>/dev/null | head -n 80";
pub const SERVICES_CMD: &str =
    "systemctl list-units --type=service --all --no-legend --no-pager --plain 2>/dev/null | head -n 120";
/// Журнал хоста.
///
/// Проверяется не наличие команды, а непустой ответ: `journalctl` бывает установлен и при
/// этом молчит, а на BusyBox журнал лежит в кольцевом буфере `logread`, о котором systemd
/// ничего не знает. Поэтому каждый источник пробуется по очереди, и берётся первый, из
/// которого что-то пришло.
pub const LOGS_CMD: &str = concat!(
    "export LANG=C.UTF-8 LC_ALL=C.UTF-8; T=''; ",
    "if command -v journalctl >/dev/null 2>&1; then ",
    "T=$(SYSTEMD_COLORS=0 journalctl -n 300 --no-pager -o short-iso 2>/dev/null); fi; ",
    "if [ -z \"$T\" ] && [ -r /var/log/syslog ]; then T=$(tail -n 300 /var/log/syslog 2>/dev/null); fi; ",
    "if [ -z \"$T\" ] && [ -r /var/log/messages ]; then T=$(tail -n 300 /var/log/messages 2>/dev/null); fi; ",
    "if [ -z \"$T\" ] && command -v logread >/dev/null 2>&1; then T=$(logread 2>/dev/null | tail -n 300); fi; ",
    "if [ -z \"$T\" ]; then T=$(dmesg 2>/dev/null | tail -n 300); fi; ",
    "if [ -n \"$T\" ]; then printf '%s\\n' \"$T\"; ",
    "else echo \"Журнал недоступен: нет journalctl, syslog, logread и dmesg\"; fi"
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

/// Проверка имени службы и действия — одна на все системы.
///
/// Имя уходит в командную строку, поэтому набор символов узкий: буквы, цифры и `-_.@:`.
/// Отдельно запрещено начинать с дефиса — иначе имя прочтётся как ключ той программы,
/// которой его передали (`rc-service` разбирает аргументы обычным способом).
pub fn check_service(name: &str, action: &str) -> Result<(), String> {
    if !matches!(action, "start" | "stop" | "restart") {
        return Err("Допустимы start / stop / restart".into());
    }
    if name.is_empty()
        || name.len() > 128
        || name.starts_with('-')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | ':'))
    {
        return Err("Некорректное имя сервиса".into());
    }
    Ok(())
}