//! Одна команда на нескольких серверах.

use crate::{actionlog, multihost, AppState};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

/// Одна команда на нескольких серверах. Результат каждого хоста уходит событием
/// сразу, как только готов; здесь возвращается общий список - для истории в окне.
#[tauri::command]
pub async fn multi_exec(
    app: AppHandle,
    state: State<'_, AppState>,
    server_ids: Vec<String>,
    command: String,
    concurrency: Option<u32>,
    timeout_sec: Option<u64>,
) -> Result<Vec<Value>, String> {
    let cancel = state.ops.begin("multi-exec");
    let opts = multihost::RunOptions::new(concurrency, timeout_sec);
    let journal_cmd = actionlog::text(&command);
    let out = multihost::run(app, server_ids, command, opts, cancel).await;
    state.ops.finish("multi-exec");
    for host in &out {
        let Some(server) = host["serverId"].as_str() else {
            continue;
        };
        let state_name = host["state"].as_str().unwrap_or("");
        let code = host["code"].as_i64();
        let result = multihost::exec_outcome(host);
        actionlog::record(
            Some(server),
            None,
            "fleet.exec",
            json!({ "command": journal_cmd, "state": state_name, "code": code }),
            result,
        );
    }
    Ok(out)
}

#[tauri::command]
pub fn multi_exec_cancel(state: State<'_, AppState>) {
    state.ops.cancel("multi-exec");
}
