//! Метрики сервера: один сборщик на SSH-сессию, последний замер и история за час.
//!
//! Раньше замер делала сама панель обзора, по таймеру раз в три секунды, и помнила только
//! последний снимок. Отсюда три беды сразу: истории не было - графики строить не из чего;
//! таймер не ждал ответа, и на медленном сервере запросы накладывались друг на друга; а
//! стоило уйти с вкладки, как о сервере переставали знать что-либо вообще.
//!
//! Теперь у каждой SSH-сессии ровно один сборщик. Пока панель смотрят, он меряет раз в три
//! секунды, пока не смотрят - раз в тридцать: этого хватает, чтобы в графике за час не было
//! дыр, и почти ничего не стоит серверу. Замеры идут строго один за другим, наложиться им не
//! на что. Панель забирает готовый последний замер и историю, а на сервер сама не ходит.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use crate::ssh::{wait_cancel, CancelRx, SharedHandle};

/// Сколько истории держим: час.
const HISTORY_SPAN_MS: u64 = 60 * 60 * 1000;
/// Потолок числа точек на случай, если замеры вдруг пойдут чаще задуманного.
const HISTORY_MAX: usize = 1500;
/// Как часто меряем, пока панель смотрят.
const WATCHED_EVERY: Duration = Duration::from_secs(3);
/// Как часто меряем, пока не смотрят.
const IDLE_EVERY: Duration = Duration::from_secs(30);
/// Сколько после последнего взгляда панели считаем, что её всё ещё смотрят.
const WATCH_GRACE: Duration = Duration::from_secs(10);

/// Точка истории: только то, из чего строятся графики и оценка здоровья.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    /// Время замера, миллисекунды эпохи.
    pub t: u64,
    pub cpu: f64,
    /// Занятая память в процентах.
    pub mem: f64,
    /// Заполненность главного тома в процентах.
    pub disk: f64,
    /// Средняя загрузка за минуту. На Windows такого понятия нет - и значения тоже.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load: Option<f64>,
    pub cores: u64,
    /// Счётчики байт интерфейса с момента его подъёма; скорость считает интерфейс.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx: Option<u64>,
}

struct Entry {
    latest: Option<Value>,
    history: VecDeque<Point>,
    watched_until: Option<Instant>,
    /// Будит сборщик, когда панель открыли посреди долгой паузы: ждать тридцать секунд
    /// первой цифры человек не должен.
    wake: Arc<tokio::sync::Notify>,
}

static STATE: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut HashMap<String, Entry>) -> T) -> T {
    let mut g = crate::sync::lock(&STATE);
    f(g.get_or_insert_with(HashMap::new))
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Заводит запись о сессии и отдаёт то, чем её сборщик будят.
fn open_entry(id: &str) -> Arc<tokio::sync::Notify> {
    with(|m| {
        m.entry(id.to_owned())
            .or_insert_with(|| Entry {
                latest: None,
                history: VecDeque::new(),
                watched_until: None,
                wake: Arc::new(tokio::sync::Notify::new()),
            })
            .wake
            .clone()
    })
}

/// Точка истории из замера. `None` - замер неудачный, добавить в историю нечего.
pub fn point_of(sample: &Value, t: u64) -> Option<Point> {
    if sample.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let num = |k: &str| sample.get(k).and_then(Value::as_f64);
    let total = num("memTotalKb").unwrap_or(0.0);
    let used = num("memUsedKb").unwrap_or(0.0);
    let windows = sample.get("platform").and_then(Value::as_str) == Some("windows");
    Some(Point {
        t,
        cpu: num("cpuPct").unwrap_or(0.0),
        mem: if total > 0.0 { used / total * 100.0 } else { 0.0 },
        disk: num("diskPct").unwrap_or(0.0),
        load: if windows {
            None
        } else {
            sample.get("load").and_then(|l| l.get(0)).and_then(Value::as_f64)
        },
        cores: sample.get("cores").and_then(Value::as_u64).unwrap_or(1).max(1),
        rx: sample.get("netRxBytes").and_then(Value::as_u64),
        tx: sample.get("netTxBytes").and_then(Value::as_u64),
    })
}

/// Кладёт замер: он становится последним, а удачный ещё и уходит в историю.
///
/// Возвращает `false`, если сессии уже нет. Запись не создаётся заново: иначе замер,
/// закончившийся после закрытия сессии, оживил бы её историю навсегда.
pub fn record(id: &str, sample: Value, t: u64) -> bool {
    with(|m| {
        let Some(e) = m.get_mut(id) else {
            return false;
        };
        if let Some(p) = point_of(&sample, t) {
            e.history.push_back(p);
        }
        while e
            .history
            .front()
            .is_some_and(|p| t.saturating_sub(p.t) > HISTORY_SPAN_MS)
            || e.history.len() > HISTORY_MAX
        {
            e.history.pop_front();
        }
        e.latest = Some(sample);
        true
    })
}

pub fn latest(id: &str) -> Option<Value> {
    with(|m| m.get(id).and_then(|e| e.latest.clone()))
}

pub fn history(id: &str) -> Vec<Point> {
    with(|m| {
        m.get(id)
            .map(|e| e.history.iter().cloned().collect())
            .unwrap_or_default()
    })
}

/// Панель смотрит: следующие секунды меряем часто, а если сборщик спал - будим.
pub fn watch(id: &str) {
    with(|m| {
        if let Some(e) = m.get_mut(id) {
            let was_idle = e.watched_until.is_none_or(|u| u <= Instant::now());
            e.watched_until = Some(Instant::now() + WATCH_GRACE);
            if was_idle {
                e.wake.notify_one();
            }
        }
    })
}

fn next_delay(id: &str) -> Duration {
    with(|m| match m.get(id).and_then(|e| e.watched_until) {
        Some(u) if u > Instant::now() => WATCHED_EVERY,
        _ => IDLE_EVERY,
    })
}

pub fn forget(id: &str) {
    with(|m| {
        m.remove(id);
    })
}

/// Один замер: команда своя у юниксов и у Windows, разбор - в `monitor` и `platform`.
pub async fn sample(
    id: &str,
    handle: &SharedHandle,
    cancel: Option<CancelRx>,
) -> Result<Value, String> {
    let (kind, _) = crate::platform::of_session(id, handle).await;
    if kind == crate::platform::Kind::Windows {
        let (_c, out, err) = crate::ssh::exec(
            handle,
            &crate::platform::ps(crate::platform::cmd::SAMPLE_WINDOWS),
            cancel,
        )
        .await?;
        if out.trim().is_empty() && !err.trim().is_empty() {
            return Ok(json!({ "ok": false, "error": err.trim() }));
        }
        return Ok(crate::platform::win::parse_sample(&out));
    }
    let (_c, out, _e) = crate::ssh::exec(handle, crate::monitor::SAMPLE_CMD, cancel).await?;
    Ok(crate::monitor::parse(&out))
}

/// Запускает сборщик сессии. Живёт, пока жива сессия.
pub fn spawn(id: String, handle: SharedHandle, cancel: CancelRx) {
    let wake = open_entry(&id);
    tokio::spawn(async move {
        loop {
            let value = match sample(&id, &handle, Some(cancel.clone())).await {
                Ok(v) => v,
                // Неудачный замер тоже важен: панель должна показать, что данных нет, а не
                // застывшую старую картинку, похожую на живую.
                Err(e) => json!({ "ok": false, "error": e }),
            };
            if *cancel.borrow() || !record(&id, value, now_ms()) {
                break;
            }
            let delay = next_delay(&id);
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = wake.notified() => {}
                _ = wait_cancel(cancel.clone()) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn замер(cpu: u64) -> Value {
        json!({
            "ok": true, "cpuPct": cpu, "memTotalKb": 1000, "memUsedKb": 250, "diskPct": 40,
            "load": [0.5, 0.4, 0.3], "cores": 4, "netRxBytes": 10, "netTxBytes": 20
        })
    }

    #[test]
    fn удачный_замер_становится_точкой_истории() {
        let p = point_of(&замер(30), 5).expect("точка");
        assert_eq!(p.cpu, 30.0);
        assert_eq!(p.mem, 25.0, "память считается процентом от всей");
        assert_eq!(p.load, Some(0.5), "берётся загрузка за минуту");
        assert_eq!((p.rx, p.tx), (Some(10), Some(20)));
    }

    #[test]
    fn у_windows_нет_средней_загрузки() {
        // Не ноль, а отсутствие: нулевая загрузка на графике выглядела бы как простой.
        let mut v = замер(1);
        v["platform"] = json!("windows");
        assert!(point_of(&v, 1).expect("точка").load.is_none());
    }

    #[test]
    fn неудачный_замер_в_историю_не_идёт_но_становится_последним() {
        let id = "тест-метрики-неудача";
        open_entry(id);
        assert!(record(id, json!({ "ok": false, "error": "нет связи" }), 1));
        assert!(history(id).is_empty());
        assert_eq!(latest(id).expect("последний")["error"], "нет связи");
        forget(id);
    }

    #[test]
    fn история_держит_только_последний_час() {
        let id = "тест-метрики-час";
        open_entry(id);
        record(id, замер(1), 0);
        record(id, замер(2), HISTORY_SPAN_MS / 2);
        record(id, замер(3), HISTORY_SPAN_MS + 1000);
        let h = history(id);
        assert_eq!(h.len(), 2, "точка старше часа уходит: {h:?}");
        assert_eq!(h[0].cpu, 2.0);
        forget(id);
    }

    #[test]
    fn после_закрытия_сессии_замер_не_оживляет_запись() {
        // Замер, закончившийся после закрытия сессии, не должен вернуть её историю.
        let id = "тест-метрики-закрыта";
        open_entry(id);
        forget(id);
        assert!(!record(id, замер(1), 1));
        assert!(latest(id).is_none());
    }
}
