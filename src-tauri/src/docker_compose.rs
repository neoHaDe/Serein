//! Docker Compose: проекты и сервисы через `docker compose` по SSH-exec.

use serde_json::{json, Value};
use std::collections::HashMap;

pub const LIST_CMD: &str = "docker compose ls -a --format json";

pub const LIST_PS_JSON_CMD: &str = "docker ps -a --filter label=com.docker.compose.project --format \"{{json .}}\"";

const ACTIONS: &[&str] = &["up", "down", "start", "stop", "restart"];

fn parse_docker_labels(raw: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in raw.split(',') {
        let mut kv = part.splitn(2, '=');
        let k = kv.next().unwrap_or("").trim();
        let v = kv.next().unwrap_or("").trim();
        if !k.is_empty() {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

fn push_project(
    projects: &mut Vec<Value>,
    by_name: &mut HashMap<String, Value>,
    name: &str,
    status: &str,
    compose_file: &str,
) {
    if name.is_empty() {
        return;
    }
    let cf = compose_file.to_string();
    let key = if cf.is_empty() { name.to_string() } else { cf.clone() };
    if by_name.contains_key(&key) {
        return;
    }
    let row = json!({
        "name": name,
        "project": name,
        "status": status,
        "composeFile": cf,
    });
    by_name.insert(key, row.clone());
    projects.push(row);
}

fn push_project_from_ls(projects: &mut Vec<Value>, by_name: &mut HashMap<String, Value>, p: &Value) {
    let name = p.get("Name").and_then(|v| v.as_str()).unwrap_or("");
    let compose_raw = p.get("ConfigFiles").and_then(|v| v.as_str()).unwrap_or("");
    let compose_file = first_compose_file(compose_raw);
    let status = p.get("Status").and_then(|v| v.as_str()).unwrap_or("");
    push_project(projects, by_name, name, status, &compose_file);
}

fn collect_json_lines(stdout: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return out;
    }
    if trimmed.starts_with('[') {
        if let Ok(arr) = serde_json::from_str::<Vec<Value>>(trimmed) {
            return arr;
        }
    }
    for line in stdout.lines() {
        let s = line.trim();
        if s.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(s) {
            out.push(v);
        }
    }
    out
}

fn push_service(services: &mut Vec<Value>, p: &Value) {
    let status = p.get("Status").and_then(|v| v.as_str()).unwrap_or("").to_string();
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
    let id = p.get("ID").and_then(|v| v.as_str()).unwrap_or("");
    let service = p
        .get("Service")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| p.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string());
    if service.is_empty() && id.is_empty() {
        return;
    }
    services.push(json!({
        "name": p.get("Name").and_then(|v| v.as_str()).unwrap_or(""),
        "service": service,
        "id": id.chars().take(12).collect::<String>(),
        "image": p.get("Image").and_then(|v| v.as_str()).unwrap_or(""),
        "state": state,
        "status": status,
        "ports": p.get("Ports").and_then(|v| v.as_str()).unwrap_or(""),
    }));
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Путь к compose-файлу для ключа `-f`.
///
/// `Ok(None)` - пути нет или он не POSIX (сервер на Windows): команда пойдёт по имени проекта,
/// так compose умеет всё, кроме `up`. Путь с управляющими знаками - отказ. Годный путь идёт
/// в кавычках целиком: раньше из него вырезались пробелы и прочие знаки, и файл в каталоге
/// с пробелом переставал находиться.
fn compose_file(p: &str) -> Result<Option<&str>, String> {
    let p = p.trim();
    if p.chars().any(char::is_control) {
        return Err(format!("Недопустимый путь к compose-файлу: «{}»", p.escape_debug()));
    }
    Ok(Some(p).filter(|p| p.starts_with('/')))
}

/// Имя проекта или службы: латиница, цифры и `_ . -`, первым знаком - буква или цифра.
/// Отказ вместо обрезки: обрезанное имя - это другой проект, а ведущий `-` compose прочёл
/// бы как ключ.
fn compose_name<'a>(what: &str, name: &'a str) -> Result<&'a str, String> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    if first_ok && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) {
        Ok(name)
    } else {
        Err(format!(
            "Недопустимое имя {what} «{}»: только латиница, цифры и . _ -, первым - буква или цифра",
            name.escape_debug()
        ))
    }
}

/// Начало любой команды над проектом: `docker compose [-f файл] -p проект`.
fn base_cmd(file: Option<&str>, project: &str) -> Result<String, String> {
    let proj = compose_name("проекта", project)?;
    Ok(match file {
        Some(f) => format!("docker compose -f {} -p {}", shell_quote(f), proj),
        None => format!("docker compose -p {proj}"),
    })
}

fn first_compose_file(raw: &str) -> String {
    raw.split(',')
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string()
}

fn guess_compose_file(working_dir: &str) -> String {
    let wd = working_dir.trim().trim_end_matches('/');
    if wd.is_empty() {
        return String::new();
    }
    format!("{wd}/docker-compose.yml")
}

pub fn parse_list(code: i32, stdout: &str, stderr: &str) -> Value {
    if code != 0 {
        let err = stderr.trim();
        let low = err.to_lowercase();
        let msg = if low.contains("not found") || low.contains("command not found") {
            "Docker Compose не установлен на сервере".to_string()
        } else if !err.is_empty() {
            err.to_string()
        } else {
            "docker compose ls завершился с ошибкой".to_string()
        };
        return json!({ "ok": false, "error": msg });
    }

    let mut projects: Vec<Value> = Vec::new();
    let mut by_name: HashMap<String, Value> = HashMap::new();
    for p in collect_json_lines(stdout) {
        push_project_from_ls(&mut projects, &mut by_name, &p);
    }
    json!({ "ok": true, "projects": projects })
}

pub fn parse_list_from_ps_json(code: i32, stdout: &str, stderr: &str) -> Value {
    if code != 0 {
        let err = stderr.trim();
        let msg = if !err.is_empty() {
            err.to_string()
        } else {
            "Не удалось найти compose-проекты".to_string()
        };
        return json!({ "ok": false, "error": msg });
    }

    let mut projects: Vec<Value> = Vec::new();
    let mut by_name: HashMap<String, Value> = HashMap::new();
    for p in collect_json_lines(stdout) {
        let labels = p.get("Labels").and_then(|v| v.as_str()).unwrap_or("");
        let map = parse_docker_labels(labels);
        let name = map.get("com.docker.compose.project").map(String::as_str).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let wd = map
            .get("com.docker.compose.project.working_dir")
            .map(String::as_str)
            .unwrap_or("");
        let cf_raw = map
            .get("com.docker.compose.project.config_files")
            .map(String::as_str)
            .unwrap_or("");
        let compose_file = {
            let cf = first_compose_file(cf_raw);
            if cf.is_empty() {
                guess_compose_file(wd)
            } else {
                cf
            }
        };
        push_project(&mut projects, &mut by_name, name, "detected", &compose_file);
    }
    json!({ "ok": true, "projects": projects })
}

pub fn merge_projects(primary: Value, fallback: Value) -> Value {
    if !primary["ok"].as_bool().unwrap_or(false) {
        return fallback;
    }
    if !fallback["ok"].as_bool().unwrap_or(false) {
        return primary;
    }
    let mut by_key: HashMap<String, Value> = HashMap::new();
    for p in primary["projects"].as_array().cloned().unwrap_or_default() {
        let key = p["composeFile"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| p["name"].as_str().unwrap_or("").to_string());
        if !key.is_empty() {
            by_key.insert(key, p);
        }
    }
    for p in fallback["projects"].as_array().cloned().unwrap_or_default() {
        let key = p["composeFile"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| p["name"].as_str().unwrap_or("").to_string());
        if key.is_empty() {
            continue;
        }
        by_key.entry(key).or_insert(p);
    }
    let mut projects: Vec<Value> = by_key.into_values().collect();
    projects.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));
    json!({ "ok": true, "projects": projects })
}

pub fn ps_cmd(file: &str, project: &str) -> Result<String, String> {
    Ok(format!(
        "{} ps -a --format json",
        base_cmd(compose_file(file)?, project)?
    ))
}

pub fn parse_ps(code: i32, stdout: &str, stderr: &str) -> Value {
    if code != 0 {
        let err = stderr.trim();
        let msg = if !err.is_empty() {
            err.to_string()
        } else {
            "docker compose ps завершился с ошибкой".to_string()
        };
        return json!({ "ok": false, "error": msg });
    }

    let mut services: Vec<Value> = Vec::new();
    for p in collect_json_lines(stdout) {
        push_service(&mut services, &p);
    }
    json!({ "ok": true, "services": services })
}

pub fn action_cmd(file: &str, project: &str, action: &str, service: Option<&str>) -> Result<String, String> {
    if !ACTIONS.contains(&action) {
        return Err(format!("Неизвестное действие: {action}"));
    }
    let file = compose_file(file)?;
    // Без `-f` compose ищет файл в текущем каталоге - на сервере это домашний каталог, и `up`
    // поднял бы оттуда чужой проект под этим именем. Остальным действиям хватает имени.
    if action == "up" && file.is_none() {
        return Err("Запустить проект можно только по пути к его compose-файлу, а он неизвестен".into());
    }
    let base = base_cmd(file, project)?;
    Ok(match action {
        "up" => format!("{base} up -d"),
        "down" => format!("{base} down"),
        _ => format!("{base} {action} {}", compose_name("службы", service.unwrap_or(""))?),
    })
}

pub fn logs_cmd(file: &str, project: &str, service: &str) -> Result<String, String> {
    let base = base_cmd(compose_file(file)?, project)?;
    let svc = compose_name("службы", service)?;
    Ok(format!("{base} logs --tail 200 -f {svc} 2>&1"))
}

pub fn read_compose_cmd(file: &str) -> Result<String, String> {
    let f = compose_file(file)?.ok_or("Путь к compose-файлу неизвестен - прочитать нечего")?;
    Ok(format!("head -n 400 {}", shell_quote(f)))
}

pub fn parse_compose_text(code: i32, stdout: &str, stderr: &str) -> Value {
    if code != 0 {
        let err = stderr.trim();
        let msg = if !err.is_empty() {
            err.to_string()
        } else {
            "Не удалось прочитать compose-файл".to_string()
        };
        return json!({ "ok": false, "error": msg });
    }
    json!({ "ok": true, "text": stdout })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_ok() {
        let out = r#"{"Name":"site","Status":"running(3)","ConfigFiles":"/srv/site/docker-compose.yml"}"#;
        let v = parse_list(0, out, "");
        assert!(v["ok"].as_bool().unwrap());
        assert_eq!(v["projects"][0]["name"], "site");
        assert_eq!(v["projects"][0]["composeFile"], "/srv/site/docker-compose.yml");
    }

    #[test]
    fn first_compose_file_multi() {
        assert_eq!(
            first_compose_file("/a/docker-compose.yml,/a/override.yml"),
            "/a/docker-compose.yml"
        );
    }

    #[test]
    fn parse_list_array_ok() {
        let out = r#"[{"Name":"site","Status":"running(3)","ConfigFiles":"/srv/site/docker-compose.yml"}]"#;
        let v = parse_list(0, out, "");
        assert!(v["ok"].as_bool().unwrap());
        assert_eq!(v["projects"][0]["name"], "site");
    }

    #[test]
    fn parse_ps_json_ok() {
        let out = r#"{"Names":"nc-app-1","Labels":"com.docker.compose.project=nextcloud-stack,com.docker.compose.project.working_dir=/srv/nextcloud-stack,com.docker.compose.service=app"}"#;
        let v = parse_list_from_ps_json(0, out, "");
        assert!(v["ok"].as_bool().unwrap());
        assert_eq!(v["projects"][0]["name"], "nextcloud-stack");
        assert_eq!(
            v["projects"][0]["composeFile"],
            "/srv/nextcloud-stack/docker-compose.yml"
        );
    }

    #[test]
    fn parse_labels_ok() {
        let out = r#"{"Names":"x","Labels":"com.docker.compose.project=site,com.docker.compose.project.config_files=/srv/site/docker-compose.yml"}"#;
        let v = parse_list_from_ps_json(0, out, "");
        assert!(v["ok"].as_bool().unwrap());
        assert_eq!(v["projects"][0]["name"], "site");
    }

    #[test]
    fn action_whitelist() {
        let f = "/srv/a/docker-compose.yml";
        assert_eq!(
            action_cmd(f, "site", "up", None).unwrap(),
            "docker compose -f '/srv/a/docker-compose.yml' -p site up -d"
        );
        assert_eq!(
            action_cmd(f, "site", "restart", Some("web")).unwrap(),
            "docker compose -f '/srv/a/docker-compose.yml' -p site restart web"
        );
        assert!(action_cmd(f, "", "up", None).is_err());
        assert!(action_cmd(f, "site", "rm", None).is_err());
    }

    #[test]
    fn путь_к_compose_с_пробелом_не_портится() {
        assert_eq!(
            ps_cmd("/srv/my app/docker-compose.yml", "site").unwrap(),
            "docker compose -f '/srv/my app/docker-compose.yml' -p site ps -a --format json"
        );
        assert_eq!(
            read_compose_cmd("/srv/it's/compose.yml").unwrap(),
            "head -n 400 '/srv/it'\\''s/compose.yml'"
        );
        assert!(ps_cmd("/srv/a\nb/compose.yml", "site").is_err());
    }

    #[test]
    fn запуск_без_пути_к_файлу_отказывает_а_не_ищет_файл_в_домашнем_каталоге() {
        assert!(action_cmd("", "site", "up", None).is_err());
        assert!(action_cmd("C:\\srv\\compose.yml", "site", "up", None).is_err());
        // Остальное compose умеет по имени проекта - так было и остаётся для Windows-серверов.
        assert_eq!(
            action_cmd("", "site", "down", None).unwrap(),
            "docker compose -p site down"
        );
        assert_eq!(
            ps_cmd("C:\\srv\\compose.yml", "site").unwrap(),
            "docker compose -p site ps -a --format json"
        );
        assert!(read_compose_cmd("").is_err());
    }

    #[test]
    fn имена_проекта_и_службы_проверяются_а_не_обрезаются() {
        let f = "/srv/a/compose.yml";
        for bad in ["site;x", "-p", "", "site x", "сайт"] {
            assert!(action_cmd(f, bad, "down", None).is_err(), "проект {bad:?}");
            assert!(action_cmd(f, "site", "restart", Some(bad)).is_err(), "служба {bad:?}");
            assert!(logs_cmd(f, "site", bad).is_err(), "журнал {bad:?}");
        }
        assert!(action_cmd(f, "site", "restart", None).is_err(), "служба обязательна");
    }
}
