//! Зашифрованный бэкап серверов, настроек, сниппетов, профилей рабочего пространства и
//! задач (порт backup.ts).
//!
//! Профили и задачи добавлены без смены версии формата: старое приложение лишние поля
//! пропустит, а старый бэкап без них читается как бэкап с пустыми списками. История запусков
//! задач сюда не входит - это журнал, а не настройка, и переносить его незачем.
//! Формат пакета совместим с Electron-версией (crypto::encrypt_with_password).

use crate::{crypto, store};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Read;

/// Бэкап несёт пароли серверов, поэтому пустой или совсем короткий пароль запрещаем:
/// иначе файл только выглядит зашифрованным.
const MIN_BACKUP_PASSWORD: usize = 8;

/// Бэкап с профилями и сниппетами должен быть небольшим. Предел проверяется во время
/// чтения, до base64 и scrypt, чтобы выбранный чужой файл не занял всю память процесса.
pub const MAX_BACKUP_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyCommandWarning {
    /// Позиция профиля внутри подписанного содержимого бэкапа.
    pub server_index: usize,
    pub server_id: Option<String>,
    pub name: String,
    pub command: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub servers: usize,
    pub snippets: usize,
    pub workspaces: usize,
    pub tasks: usize,
    pub keys_remapped: usize,
    pub proxy_commands: Vec<ProxyCommandWarning>,
    /// Связывает подтверждение с теми байтами, которые пользователь просмотрел.
    pub content_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub servers: usize,
    pub snippets: usize,
    pub workspaces: usize,
    pub tasks: usize,
    pub keys_remapped: usize,
    pub proxy_commands: Vec<ProxyCommandWarning>,
    pub proxy_commands_enabled: usize,
}

struct DecodedBackup {
    servers: Vec<Value>,
    snippets: Vec<Value>,
    workspaces: Vec<Value>,
    tasks: Vec<Value>,
    settings: Option<Value>,
}

pub fn export(password: &str) -> Result<String, String> {
    if password.chars().count() < MIN_BACKUP_PASSWORD {
        return Err(format!(
            "Пароль бэкапа - минимум {MIN_BACKUP_PASSWORD} символов: файл содержит пароли серверов"
        ));
    }
    let payload = json!({
        "version": 1,
        "exportedAt": chrono_now(),
        "servers": store::list_servers_with_secrets(),
        "settings": store::settings_get(),
        "snippets": store::snippets_list(),
        "workspaces": store::workspaces_list(),
        "tasks": store::tasks_list(),
    });
    crypto::encrypt_with_password(&payload.to_string(), password)
}

/// Прочитать выбранный файл с жёстким пределом. `take` оставляет предел действующим,
/// даже если файл подменили или дописали после metadata.
pub fn read_file(path: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_BACKUP_BYTES {
        return Err(format!(
            "Файл бэкапа больше допустимых {} МиБ",
            MAX_BACKUP_BYTES / 1024 / 1024
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BACKUP_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BACKUP_BYTES {
        return Err(format!(
            "Файл бэкапа больше допустимых {} МиБ",
            MAX_BACKUP_BYTES / 1024 / 1024
        ));
    }
    String::from_utf8(bytes).map_err(|_| "Файл бэкапа должен быть текстом UTF-8".to_owned())
}

fn decode(content: &str, password: &str) -> Result<DecodedBackup, String> {
    let json = crypto::decrypt_with_password(content, password)
        .map_err(|_| "Неверный пароль или повреждённый файл бэкапа".to_string())?;
    let payload: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    from_payload(&payload)
}

/// Содержимое расшифрованного бэкапа. Отсутствующий список - пустой: так читаются бэкапы,
/// сделанные до появления профилей и задач.
fn from_payload(payload: &Value) -> Result<DecodedBackup, String> {
    if payload.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return Err("Неподдерживаемая версия бэкапа".into());
    }
    let list = |key: &str| payload.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Ok(DecodedBackup {
        servers: list("servers"),
        snippets: list("snippets"),
        workspaces: list("workspaces"),
        tasks: list("tasks"),
        settings: payload.get("settings").cloned(),
    })
}

fn proxy_commands(servers: &[Value]) -> Vec<ProxyCommandWarning> {
    servers
        .iter()
        .enumerate()
        .filter_map(|(server_index, s)| {
            let command = s.get("proxyCommand")?.as_str()?.trim();
            if command.is_empty() {
                return None;
            }
            Some(ProxyCommandWarning {
                server_index,
                server_id: s.get("id").and_then(|v| v.as_str()).map(str::to_owned),
                name: s
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("без имени")
                    .to_owned(),
                command: command.to_owned(),
            })
        })
        .collect()
}

pub fn preview(content: &str, password: &str) -> Result<ImportPreview, String> {
    let decoded = decode(content, password)?;
    let mut keys_remapped = 0usize;
    for s in &decoded.servers {
        let (_, fixed) = remap_key_path(s.clone());
        if fixed {
            keys_remapped += 1;
        }
    }
    Ok(ImportPreview {
        servers: decoded.servers.len(),
        snippets: decoded.snippets.len(),
        workspaces: decoded.workspaces.len(),
        tasks: decoded.tasks.len(),
        keys_remapped,
        proxy_commands: proxy_commands(&decoded.servers),
        content_sha256: content_hash(content),
    })
}

/// Применить уже просмотренный бэкап. ProxyCommand сохраняются только для конкретных
/// отмеченных позиций; остальные удаляются до передачи профиля в хранилище.
pub fn import(
    content: &str,
    password: &str,
    expected_sha256: &str,
    accepted_proxy_commands: &[usize],
) -> Result<ImportResult, String> {
    if content_hash(content) != expected_sha256 {
        return Err("Файл бэкапа изменился после предпросмотра; откройте его заново".into());
    }
    let decoded = decode(content, password)?;
    let warnings = proxy_commands(&decoded.servers);
    let accepted: HashSet<usize> = accepted_proxy_commands.iter().copied().collect();
    let mut keys_remapped = 0usize;
    let mut proxy_commands_enabled = 0usize;
    for (index, s) in decoded.servers.iter().enumerate() {
        let (s, fixed, proxy_enabled) = prepare_server(s.clone(), index, &accepted);
        if fixed {
            keys_remapped += 1;
        }
        if proxy_enabled {
            proxy_commands_enabled += 1;
        }
        store::servers_save(s)?;
    }
    for s in &decoded.snippets {
        store::snippets_save(s.clone())?;
    }
    for w in &decoded.workspaces {
        store::workspaces_save(w.clone())?;
    }
    // Задачи несут команды, но в отличие от ProxyCommand сами не запускаются никогда: только
    // по кнопке, после подтверждения, и пробный прогон показывает их до запуска.
    for t in &decoded.tasks {
        store::tasks_save(t.clone())?;
    }
    if let Some(settings) = decoded.settings {
        store::settings_set(settings)?;
    }
    Ok(ImportResult {
        servers: decoded.servers.len(),
        snippets: decoded.snippets.len(),
        workspaces: decoded.workspaces.len(),
        tasks: decoded.tasks.len(),
        keys_remapped,
        proxy_commands: warnings,
        proxy_commands_enabled,
    })
}

fn content_hash(content: &str) -> String {
    hex::encode(Sha256::digest(content.as_bytes()))
}

fn prepare_server(server: Value, index: usize, accepted: &HashSet<usize>) -> (Value, bool, bool) {
    let (mut server, key_remapped) = remap_key_path(server);
    let has_proxy = server
        .get("proxyCommand")
        .and_then(|v| v.as_str())
        .is_some_and(|v| !v.trim().is_empty());
    let proxy_enabled = has_proxy && accepted.contains(&index);
    if !proxy_enabled {
        if let Some(obj) = server.as_object_mut() {
            obj.remove("proxyCommand");
        }
    }
    (server, key_remapped, proxy_enabled)
}

/// Подставить в профиль путь к ключу, который существует на этой системе.
///
/// Бэкап с Windows несёт абсолютный путь вида `C:\Users\…\.ssh\id_ed25519`. На Linux
/// такого файла нет, и подключение падало на ровном месте. Правим только когда файл
/// действительно нашёлся: переписать путь на другой, столь же несуществующий, - значит
/// соврать пользователю и спрятать причину.
fn remap_key_path(mut s: Value) -> (Value, bool) {
    let Some(o) = s.as_object_mut() else {
        return (s, false);
    };
    let Some(raw) = o
        .get("privateKeyPath")
        .and_then(|v| v.as_str())
        .map(|x| x.to_string())
    else {
        return (s, false);
    };
    if raw.trim().is_empty() {
        return (s, false);
    }
    let fixed = crate::paths::resolve_identity(&raw);
    if fixed != raw && std::path::Path::new(&fixed).exists() {
        o.insert("privateKeyPath".into(), json!(fixed));
        return (s, true);
    }
    (s, false)
}

// Лёгкая ISO-метка без зависимости от chrono.
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("@{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_commands_are_listed_with_stable_positions() {
        let servers = vec![
            json!({"id": "plain", "name": "Обычный"}),
            json!({"id": "jump", "name": "Через команду", "proxyCommand": "  nc %h %p  "}),
        ];
        let warnings = proxy_commands(&servers);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].server_index, 1);
        assert_eq!(warnings[0].server_id.as_deref(), Some("jump"));
        assert_eq!(warnings[0].command, "nc %h %p");
    }

    #[test]
    fn старый_бэкап_без_профилей_и_задач_читается() {
        let old = json!({ "version": 1, "servers": [{ "id": "a" }], "snippets": [] });
        let d = from_payload(&old).expect("старый формат");
        assert_eq!((d.servers.len(), d.workspaces.len(), d.tasks.len()), (1, 0, 0));

        let new = json!({
            "version": 1,
            "servers": [],
            "workspaces": [{ "id": "w", "name": "Продакшн", "tabs": [], "savedAt": 1 }],
            "tasks": [{ "id": "t", "name": "Выкладка", "steps": [], "serverIds": [] }, { "id": "u", "name": "Логи", "steps": [], "serverIds": [] }],
        });
        let d = from_payload(&new).expect("новый формат");
        assert_eq!((d.workspaces.len(), d.tasks.len()), (1, 2));
        assert!(from_payload(&json!({ "version": 2 })).is_err());
    }

    #[test]
    fn content_hash_changes_when_previewed_file_changes() {
        assert_ne!(content_hash("backup-a"), content_hash("backup-b"));
    }

    #[test]
    fn proxy_command_is_disabled_until_its_exact_position_is_accepted() {
        let server = json!({"id": "jump", "proxyCommand": "nc %h %p"});
        let (disabled, _, enabled) = prepare_server(server.clone(), 3, &HashSet::new());
        assert!(!enabled);
        assert!(disabled.get("proxyCommand").is_none());

        let accepted = HashSet::from([3]);
        let (enabled_server, _, enabled) = prepare_server(server, 3, &accepted);
        assert!(enabled);
        assert_eq!(enabled_server["proxyCommand"], "nc %h %p");
    }
}
