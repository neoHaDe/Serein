//! Базы данных через SSH-сессию.

use crate::{actionlog, db, policy, AppState};
use serde_json::{json, Value};
use tauri::State;

/// Подключается к базе данных рядом с сервером.
///
/// Через ту же SSH-сессию, а не отдельным соединением: база слушает петлю сервера и в сеть
/// не смотрит. Иначе пришлось бы вручную поднимать проброс порта и помнить, что он открыт.
#[tauri::command]
pub async fn db_open(state: State<'_, AppState>, session_id: String, params: db::Params) -> Result<Value, String> {
    policy::check_target(params.host(), "база данных")?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let id = format!("db-{}", uuid::Uuid::new_v4());
    let journal = json!({ "database": params.database, "user": params.user });
    let r = db::open(id, &session_id, &s.handle, params).await;
    let mut detail = r.as_ref().ok().cloned().unwrap_or_else(|| json!({}));
    if let (Some(d), Some(j)) = (detail.as_object_mut(), journal.as_object()) {
        d.extend(j.clone());
    }
    actionlog::record_session(&session_id, "db.open", detail, &r);
    r
}

/// Выполняет запрос: SQL для PostgreSQL, команду для Redis.
#[tauri::command]
pub async fn db_query(id: String, text: String) -> Result<Value, String> {
    let r = db::query(&id, &text).await;
    if let Some(session) = db::session_of(&id) {
        actionlog::record_session(
            &session,
            "db.query",
            json!({ "query": actionlog::text(&text), "db": id }),
            &r,
        );
    }
    r
}

/// Останавливает выполняющийся запрос. PostgreSQL отменяет его, не закрывая соединение;
/// у остальных баз соединение закрывается - ответ по нему остался недочитанным.
#[tauri::command]
pub async fn db_cancel(id: String) -> bool {
    db::cancel(&id)
}

/// Закрытие идёт асинхронной командой не для красоты: деструктор SSH-канала обращается
/// к рантайму Tokio, и с главного потока это роняло всё приложение целиком.
#[tauri::command]
pub async fn db_close(id: String) {
    db::close(&id);
}

/// Что за база уже открыта в этой сессии, если открыта.
///
/// Панель спрашивает об этом, когда не помнит ничего сама: в откреплённом окне своя
/// память, а соединение общее и живёт в приложении.
#[tauri::command]
pub async fn db_current(session_id: String) -> Option<Value> {
    db::for_session(&session_id)
}
