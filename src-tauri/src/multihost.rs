//! Одна команда сразу на нескольких серверах.
//!
//! Смысл — не «сэкономить вкладки», а увидеть **разницу** между машинами: где пакет уже
//! обновлён, а где нет; где сервис поднят, а где упал. Поэтому результат по каждому хосту
//! отдаётся отдельно и целиком, включая код возврата, а не сливается в общий поток.
//!
//! Две вещи решены осознанно и в пользу осторожности.
//!
//! **Хосты с неподтверждённым ключом пропускаются.** Обычное подключение при первой встрече
//! спрашивает про отпечаток; здесь спрашивать некого — иначе на десяти новых серверах
//! пользователь получит десять окон подряд и нажмёт «да» не глядя. Молча доверять тоже
//! нельзя. Поэтому такой хост честно помечается пропущенным: подключитесь к нему один раз
//! обычным способом, подтвердите ключ, и он попадёт в общий прогон.
//!
//! **Соединение поднимается своё, существующая сессия не переиспользуется.** Так результат
//! не зависит от того, что происходит в открытой вкладке, — там может идти интерактивная
//! команда, свой каталог, свой `sudo`.
//!
//! ⚠ Прежняя заметка здесь винила SSH-стек: мол, любое обращение к нему из тестового
//! бинаря ломает его на загрузке. Причина оказалась другой и к SSH отношения не имела —
//! в тестовый бинарь не встраивался манифест ComCtl32 v6, и падало всё, что упоминает
//! `tauri::AppHandle`. Починено в `build.rs`; SSH-слой теперь покрывается тестами.

use crate::{knownhosts, ssh, store};
use serde_json::{json, Value};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

/// Сколько хостов обрабатываем одновременно.
///
/// Не «побольше»: каждое соединение — это рукопожатие и аутентификация, а на общем канале
/// десяток одновременных подключений начинает мешать сам себе. Четыре даёт заметный выигрыш
/// и остаётся предсказуемым.
const CONCURRENCY: usize = 4;

/// Хост, к которому не станем подключаться, и почему.
fn skip_reason(chain: &[Value]) -> Option<String> {
    // Проверяем всю цепочку: незнакомый jump-хост опаснее незнакомой цели.
    let known: Vec<String> = knownhosts::list()
        .into_iter()
        .filter_map(|e| e.get("host").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    for link in chain {
        let host = link.get("host").and_then(|v| v.as_str()).unwrap_or("");
        let port = link
            .get("port")
            .and_then(|v| v.as_u64())
            .filter(|p| *p > 0 && *p <= 65535)
            .unwrap_or(22) as u16;
        let id = knownhosts::host_id(host, port);
        if !known.iter().any(|k| k == &id) {
            return Some(format!(
                "ключ хоста {id} не подтверждён — подключитесь к нему обычным способом один раз"
            ));
        }
    }
    None
}

fn skipped(server_id: &str, name: &str, why: String) -> Value {
    json!({
        "serverId": server_id,
        "name": name,
        "state": "skipped",
        "error": why,
    })
}

fn failed(server_id: &str, name: &str, why: String, ms: u128) -> Value {
    json!({
        "serverId": server_id,
        "name": name,
        "state": "failed",
        "error": why,
        "ms": ms,
    })
}

/// Сколько секунд ждём exec на одном хосте в массовом прогоне.
const HOST_EXEC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Выполнить команду на одном сервере: подключение, exec, разрыв.
async fn run_one(server_id: String, command: String, cancel: ssh::CancelRx) -> Value {
    let name = store::servers_list()
        .into_iter()
        .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(&server_id))
        .and_then(|s| s.get("name").and_then(|v| v.as_str()).map(|x| x.to_string()))
        .unwrap_or_else(|| server_id.clone());

    let chain = match crate::resolve_chain_for(&server_id) {
        Ok(c) => c,
        Err(e) => return skipped(&server_id, &name, e),
    };
    if chain.first().and_then(|s| s.get("connection")).and_then(|v| v.as_str())
        == Some("serial")
    {
        return skipped(&server_id, &name, "COM-порт: выполнение команд не поддерживается".into());
    }
    if let Some(why) = skip_reason(&chain) {
        return skipped(&server_id, &name, why);
    }

    let started = Instant::now();
    let handle = match ssh::connect_client(chain).await {
        Ok(h) => h,
        Err(e) => return failed(&server_id, &name, e.to_string(), started.elapsed().as_millis()),
    };
    match ssh::exec_timed(&handle, &command, Some(cancel), HOST_EXEC_TIMEOUT).await {
        Ok((code, out, err)) => json!({
            "serverId": server_id,
            "name": name,
            "state": "done",
            "code": code,
            "stdout": out,
            "stderr": err,
            "ms": started.elapsed().as_millis(),
        }),
        Err(e) => failed(&server_id, &name, e, started.elapsed().as_millis()),
    }
}

/// Прогнать команду по списку серверов. Результат каждого хоста уходит событием
/// `multi-exec-result` сразу, как только готов, — на десяти машинах ждать общего конца
/// незачем, да и видно, кто отвечает медленно.
pub async fn run(
    app: AppHandle,
    server_ids: Vec<String>,
    command: String,
    cancel: ssh::CancelRx,
) -> Vec<Value> {
    use futures::stream::{FuturesUnordered, StreamExt};

    let command = command.trim().to_string();
    if command.is_empty() {
        return Vec::new();
    }

    let total = server_ids.len();
    let mut queue = server_ids.into_iter();
    let mut running = FuturesUnordered::new();
    let mut results: Vec<Value> = Vec::with_capacity(total);

    for _ in 0..CONCURRENCY.min(total) {
        if let Some(id) = queue.next() {
            running.push(run_one(id, command.clone(), cancel.clone()));
        }
    }
    while let Some(res) = running.next().await {
        let _ = app.emit(
            "multi-exec-result",
            json!({ "done": results.len() + 1, "total": total, "result": res }),
        );
        results.push(res);
        if let Some(id) = queue.next() {
            running.push(run_one(id, command.clone(), cancel.clone()));
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_host_is_skipped_with_a_reason() {
        // Хост, которого заведомо нет в known_hosts: прогон обязан его пропустить,
        // а не подключиться молча, доверившись первому встречному ключу.
        let chain = vec![json!({ "host": "no-such-host.invalid", "port": 22 })];
        let why = skip_reason(&chain).expect("незнакомый хост должен быть пропущен");
        assert!(why.contains("no-such-host.invalid"), "{why}");
        assert!(why.contains("не подтверждён"), "{why}");
    }

    #[test]
    fn skip_checks_the_whole_jump_chain() {
        // Цель может быть знакомой, а вот jump-хост — нет; это опаснее, через него идёт всё.
        let chain = vec![
            json!({ "host": "target.invalid", "port": 22 }),
            json!({ "host": "jump.invalid", "port": 2222 }),
        ];
        assert!(skip_reason(&chain).is_some());
    }

    #[test]
    fn unknown_host_is_skipped_without_touching_the_network() {
        // Прогон на незнакомом хосте обязан закончиться пропуском, а не попыткой
        // подключения: иначе смысл проверки known_hosts теряется.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("рантайм");
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let started = Instant::now();
        let res = rt.block_on(run_one("нет-такого-сервера".into(), "uptime".into(), rx));
        assert_eq!(res["state"], "skipped");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "пропуск должен быть мгновенным, без похода в сеть"
        );
    }

    #[test]
    fn result_shapes_carry_what_the_ui_needs() {
        let s = skipped("id1", "прод", "нет ключа".into());
        assert_eq!(s["state"], "skipped");
        assert_eq!(s["name"], "прод");
        let f = failed("id2", "тест", "сеть".into(), 120);
        assert_eq!(f["state"], "failed");
        assert_eq!(f["ms"], 120);
    }
}
