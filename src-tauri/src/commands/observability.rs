//! Наблюдаемость: журнал действий, политика администратора, сведения о хосте и метрики.

use crate::{actionlog, metrics, platform, policy, ssh, store, sysinfo, AppState};
use serde_json::Value;
use tauri::State;

/// Последние записи журнала действий, новые первыми.
#[tauri::command]
pub async fn action_log_list(limit: Option<usize>) -> Result<Vec<Value>, String> {
    let limit = limit.unwrap_or(1000).clamp(1, 20_000);
    tauri::async_runtime::spawn_blocking(move || actionlog::list(limit))
        .await
        .map_err(|e| e.to_string())
}

/// Проверка цепочки журнала: номера подряд, хеши сходятся.
#[tauri::command]
pub async fn action_log_verify() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(actionlog::verify)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn action_log_export(path: String) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || actionlog::export(&path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn action_log_status() -> Value {
    actionlog::status()
}

/// Что задано политикой администратора: источники, запертые настройки, запреты, ошибка.
#[tauri::command]
pub fn policy_status() -> Value {
    policy::status()
}

/// Железо сервера: процессор, видео, память, виртуализация.
///
/// Собирается один раз за сессию и запоминается: модель процессора не меняется, а панель
/// обзора обновляется каждые несколько секунд - спрашивать это по таймеру значило бы
/// впустую гонять канал.
#[tauri::command]
pub async fn session_sysinfo(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    if let Some(v) = sysinfo_cached(&id) {
        return Ok(v);
    }
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&id, &s.handle).await;
    // Команда разная, разбор один: сценарий для Windows отвечает теми же строками.
    let cmd = if kind == platform::Kind::Windows {
        platform::ps(sysinfo::CMD_WINDOWS)
    } else {
        sysinfo::CMD.to_owned()
    };
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    let v = sysinfo::parse(&out);
    crate::sync::lock(&SYSINFO)
        .get_or_insert_with(Default::default)
        .insert(id, v.clone());
    Ok(v)
}

/// Запомненные сведения о железе. Чистятся вместе с сессией.
static SYSINFO: std::sync::Mutex<Option<std::collections::HashMap<String, Value>>> = std::sync::Mutex::new(None);

fn sysinfo_cached(id: &str) -> Option<Value> {
    crate::sync::lock(&SYSINFO)
        .get_or_insert_with(Default::default)
        .get(id)
        .cloned()
}

pub(crate) fn sysinfo_forget(id: &str) {
    crate::sync::lock(&SYSINFO)
        .get_or_insert_with(Default::default)
        .remove(id);
}

#[tauri::command]
pub async fn session_monitor(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let s = state.ssh(&id).ok_or("Сессия не подключена")?;
    // Замер делает сборщик сессии (`metrics`), панель только забирает готовое. Отметка
    // «смотрят» переводит сборщик на частые замеры и будит его, если он спал.
    metrics::watch(&id);
    if let Some(v) = metrics::latest(&id) {
        return Ok(v);
    }
    // Сборщик ещё не успел сделать первый замер - делаем его сами, чтобы панель не ждала.
    let v = metrics::sample(&id, &s.handle, Some(s.cancel.subscribe())).await?;
    let _ = metrics::record(&id, v.clone(), metrics::now_ms());
    Ok(v)
}

/// История замеров сессии за последний час - для графиков обзора.
#[tauri::command]
pub fn session_metrics_history(id: String) -> Vec<metrics::Point> {
    metrics::history(&id)
}

/// Свои пороги здоровья сервера этой сессии - из его профиля. `null`, если своих нет.
///
/// По сессии, а не по номеру сервера из интерфейса: откреплённое окно знает только сессию.
#[tauri::command]
pub fn session_health_thresholds(state: State<'_, AppState>, id: String) -> Value {
    let Some(s) = state.ssh(&id) else {
        return Value::Null;
    };
    store::servers_list()
        .into_iter()
        .find(|srv| srv.get("id").and_then(|v| v.as_str()) == Some(s.server_id.as_str()))
        .and_then(|srv| srv.get("healthThresholds").cloned())
        .unwrap_or(Value::Null)
}
