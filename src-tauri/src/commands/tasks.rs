//! Сценарии: хранение, выгрузка, запуск и отмена.

use crate::{actionlog, store, tasks, AppState};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

#[tauri::command]
pub fn tasks_list() -> Vec<Value> {
    store::tasks_list()
}

/// Сохранение не требует готовой задачи - черновик без серверов тоже сохраняется. Но форма
/// должна разбираться: иначе сохранили бы то, что потом не запустится вовсе.
#[tauri::command]
pub fn tasks_save(mut t: Value) -> Result<Value, String> {
    // Значения запуска и секреты в файл задачи не пишутся: секреты только спрашивают.
    tasks::sanitize_for_save(&mut t);
    let parsed: tasks::Task = serde_json::from_value(t.clone()).map_err(|e| format!("Задача не разобралась: {e}"))?;
    if parsed.name.trim().is_empty() {
        return Err("У задачи нет названия".into());
    }
    store::tasks_save(t)
}

/// Выгрузить задачу файлом: без секретов, серверы - адресом и именем.
#[tauri::command]
pub fn tasks_export(id: String, path: String) -> Result<(), String> {
    let raw = store::tasks_list()
        .into_iter()
        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
        .ok_or("Задача не найдена")?;
    let task: tasks::Task = serde_json::from_value(raw).map_err(|e| format!("Задача не разобралась: {e}"))?;
    let out = tasks::export_task(&task, &store::servers_list_safe());
    let text = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("не записать {path}: {e}"))
}

/// Загрузить задачу из файла. Серверы находятся по адресу и имени; ненайденные - в `missing`.
#[tauri::command]
pub fn tasks_import(path: String) -> Result<Value, String> {
    let size = std::fs::metadata(&path)
        .map_err(|e| format!("не прочитать {path}: {e}"))?
        .len();
    if size > 4 * 1024 * 1024 {
        return Err("файл слишком большой для задачи".into());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("не прочитать {path}: {e}"))?;
    let file: Value = serde_json::from_str(&text).map_err(|_| "файл не JSON".to_owned())?;
    let (mut body, missing) = tasks::import_task(&file, &store::servers_list_safe())?;
    let taken: Vec<String> = store::tasks_list()
        .iter()
        .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(str::to_owned))
        .collect();
    if let Some(name) = body.get("name").and_then(|v| v.as_str()).map(str::to_owned) {
        body["name"] = json!(tasks::import_name(&name, &taken));
    }
    let saved = store::tasks_save(body)?;
    Ok(json!({ "task": saved, "missing": missing }))
}

#[tauri::command]
pub fn tasks_delete(id: String) -> Result<(), String> {
    store::tasks_delete(&id)
}

#[tauri::command]
pub fn task_runs_list() -> Vec<Value> {
    store::task_runs_list()
}

/// Запуск или пробный прогон задачи. Ход - событиями `task-progress`, в конце - отчёт.
#[tauri::command]
pub async fn tasks_run(
    app: AppHandle,
    state: State<'_, AppState>,
    task: Value,
    run_id: String,
    dry_run: bool,
) -> Result<Value, String> {
    let task: tasks::Task = serde_json::from_value(task).map_err(|e| format!("Задача не разобралась: {e}"))?;
    let key = format!("task:{run_id}");
    let cancel = state.ops.begin(&key);
    let journal_task = task.name.clone();
    let journal_profile = task.run_profile.clone();
    let out = tasks::run(app, task, run_id, dry_run, cancel).await;
    state.ops.finish(&key);
    match &out {
        Ok(report) => {
            for srv in report["servers"].as_array().into_iter().flatten() {
                let Some(server) = srv["serverId"].as_str() else {
                    continue;
                };
                let st = srv["state"].as_str().unwrap_or("");
                let result = tasks::run_outcome(srv);
                actionlog::record(
                    Some(server),
                    None,
                    "task.run",
                    json!({ "task": journal_task, "profile": journal_profile, "dryRun": dry_run, "state": st, "errors": srv["errors"] }),
                    result,
                );
            }
        }
        Err(e) => actionlog::record(
            None,
            None,
            "task.run",
            json!({ "task": journal_task, "profile": journal_profile, "dryRun": dry_run }),
            Err(e.clone()),
        ),
    }
    out
}

#[tauri::command]
pub fn tasks_cancel(state: State<'_, AppState>, run_id: String) {
    state.ops.cancel(&format!("task:{run_id}"));
}
