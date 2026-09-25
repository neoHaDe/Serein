//! Профиль: настройки, серверы, сниппеты, раскладка, импорт, мастер-пароль, бэкап, ключи.

use crate::{actionlog, backup, importers, keygen, policy, ssh, store, vault, AppState};
use serde_json::{json, Value};
use tauri::State;

/// scrypt специально дорогой. Один процесс не должен одновременно запускать несколько
/// операций с мастер-паролем/бэкапом и умножать их расход памяти.
static PASSWORD_KDF_GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[tauri::command]
pub fn settings_get() -> Value {
    store::settings_get()
}

#[tauri::command]
pub fn settings_set(state: State<'_, AppState>, patch: Value) -> Result<Value, String> {
    let cur = store::settings_set(patch)?;
    actionlog::configure(&cur);
    if let Some(n) = cur.get("sftpConcurrency").and_then(|v| v.as_u64()) {
        state.transfers.set_limit(n as usize);
    }
    Ok(cur)
}

#[tauri::command]
pub fn servers_list() -> Vec<Value> {
    store::servers_list_safe()
}

#[tauri::command]
pub fn servers_save(mut cfg: Value) -> Result<Value, String> {
    let has_secret = ["password", "passphrase"]
        .iter()
        .any(|k| cfg.get(*k).and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty()));
    if policy::forbids_saved_passwords() {
        // Не храним вовсе: пароль спросится при подключении.
        if let Some(o) = cfg.as_object_mut() {
            o.remove("password");
            o.remove("passphrase");
        }
    } else if has_secret && policy::requires_master_password() && !vault::is_enabled() {
        return Err(
            "Политика администратора требует мастер-пароль: включите его в настройках, прежде чем сохранять пароли"
                .into(),
        );
    }
    store::servers_save(cfg)
}

/// Перестановка серверов после перетаскивания: `[{ id, group, order }]`.
#[tauri::command]
pub fn servers_reorder(items: Vec<Value>) -> Result<(), String> {
    store::servers_reorder(&items)
}

#[tauri::command]
pub fn servers_delete(id: String) -> Result<(), String> {
    store::servers_delete(&id)
}

#[tauri::command]
pub fn snippets_list() -> Vec<Value> {
    store::snippets_list()
}

#[tauri::command]
pub fn snippets_save(s: Value) -> Result<Value, String> {
    store::snippets_save(s)
}

#[tauri::command]
pub fn snippets_delete(id: String) -> Result<(), String> {
    store::snippets_delete(&id)
}

#[tauri::command]
pub fn workspaces_list() -> Vec<Value> {
    store::workspaces_list()
}

#[tauri::command]
pub fn workspaces_save(p: Value) -> Result<Value, String> {
    store::workspaces_save(p)
}

#[tauri::command]
pub fn workspaces_delete(id: String) -> Result<(), String> {
    store::workspaces_delete(&id)
}

#[tauri::command]
pub fn layout_get() -> Value {
    store::layout_get()
}

#[tauri::command]
pub fn layout_set(tabs: Value) -> Result<(), String> {
    store::layout_set(tabs)
}

#[tauri::command]
pub fn aux_layout_get() -> Value {
    store::aux_layout_get()
}

#[tauri::command]
pub fn aux_layout_set(layout: Value) -> Result<(), String> {
    store::aux_layout_set(layout)
}

#[tauri::command]
pub fn vault_status() -> Value {
    vault::status()
}

#[tauri::command]
pub async fn vault_unlock(password: String) -> bool {
    let Ok(_permit) = PASSWORD_KDF_GATE.acquire().await else {
        return false;
    };
    tokio::task::spawn_blocking(move || vault::unlock(&password))
        .await
        .unwrap_or(false)
}

#[tauri::command]
pub async fn vault_enable(password: String) -> Value {
    let Ok(_permit) = PASSWORD_KDF_GATE.acquire().await else {
        return json!({ "ok": false, "error": "Очередь операций с паролем закрыта" });
    };
    tokio::task::spawn_blocking(move || vault::enable(&password))
        .await
        .unwrap_or_else(|e| json!({ "ok": false, "error": format!("Операция прервана: {e}") }))
}

#[tauri::command]
pub async fn vault_disable(password: String) -> Value {
    if policy::requires_master_password() {
        return json!({ "ok": false, "error": "Мастер-пароль обязателен по политике администратора" });
    }
    let Ok(_permit) = PASSWORD_KDF_GATE.acquire().await else {
        return json!({ "ok": false, "error": "Очередь операций с паролем закрыта" });
    };
    tokio::task::spawn_blocking(move || vault::disable(&password))
        .await
        .unwrap_or_else(|e| json!({ "ok": false, "error": format!("Операция прервана: {e}") }))
}

#[tauri::command]
pub async fn backup_export(password: String, path: String) -> Result<Value, String> {
    let _permit = PASSWORD_KDF_GATE
        .acquire()
        .await
        .map_err(|_| "Очередь операций с бэкапом закрыта".to_owned())?;
    tokio::task::spawn_blocking(move || {
        let content = backup::export(&password)?;
        if content.len() as u64 > backup::MAX_BACKUP_BYTES {
            return Err("Бэкап превышает допустимые 32 МиБ".to_owned());
        }
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
        Ok(json!({ "saved": true, "path": path }))
    })
    .await
    .map_err(|e| format!("Операция с бэкапом прервана: {e}"))?
}

#[tauri::command]
pub async fn backup_preview(password: String, path: String) -> Result<Value, String> {
    let _permit = PASSWORD_KDF_GATE
        .acquire()
        .await
        .map_err(|_| "Очередь операций с бэкапом закрыта".to_owned())?;
    tokio::task::spawn_blocking(move || {
        let content = backup::read_file(&path)?;
        let preview = backup::preview(&content, &password)?;
        serde_json::to_value(preview).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Предпросмотр бэкапа прерван: {e}"))?
}

#[tauri::command]
pub async fn backup_import(
    password: String,
    path: String,
    expected_sha256: String,
    accepted_proxy_commands: Vec<usize>,
) -> Result<Value, String> {
    let _permit = PASSWORD_KDF_GATE
        .acquire()
        .await
        .map_err(|_| "Очередь операций с бэкапом закрыта".to_owned())?;
    tokio::task::spawn_blocking(move || {
        let content = backup::read_file(&path)?;
        let counts = backup::import(&content, &password, &expected_sha256, &accepted_proxy_commands)?;
        let mut value = serde_json::to_value(counts).map_err(|e| e.to_string())?;
        value["imported"] = json!(true);
        Ok(value)
    })
    .await
    .map_err(|e| format!("Импорт бэкапа прерван: {e}"))?
}

#[tauri::command]
pub fn keygen_generate(params: Value) -> Result<Value, String> {
    keygen::generate(&params)
}

#[tauri::command]
pub fn keygen_save(path: String, key: Value) -> Result<Value, String> {
    keygen::save_to(&path, &key)
}

#[tauri::command]
pub async fn keygen_install(
    state: State<'_, AppState>,
    session_id: String,
    public_key: String,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (code, _o, err) = ssh::exec(&s.handle, &keygen::install_cmd(&public_key), Some(s.cancel.subscribe())).await?;
    ssh::exit_result(code, &err, || format!("Код {code}"))?;
    Ok(json!({ "installed": true }))
}

#[tauri::command]
pub fn servers_import_ssh_config() -> Result<Value, String> {
    let r = importers::import_ssh_config()?;
    Ok(json!({ "imported": r.imported, "unresolvedJumps": r.unresolved_jumps }))
}

#[tauri::command]
pub fn servers_import_putty() -> Result<Value, String> {
    Ok(json!({ "imported": importers::import_putty()? }))
}

#[tauri::command]
pub fn servers_import_mobaxterm() -> Result<Value, String> {
    Ok(json!({ "imported": importers::import_mobaxterm()? }))
}

#[tauri::command]
pub fn servers_import_xshell() -> Result<Value, String> {
    Ok(json!({ "imported": importers::import_xshell()? }))
}

#[tauri::command]
pub fn servers_import_securecrt() -> Result<Value, String> {
    Ok(json!({ "imported": importers::import_securecrt()? }))
}
