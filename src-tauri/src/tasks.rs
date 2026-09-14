//! Задачи: упорядоченные шаги на выбранных серверах.
//!
//! Задача - это то, что человек иначе делает руками по одной вкладке: залить сборку,
//! перезапустить службу, дождаться, пока сервис ответит, и так на каждом сервере. Шаги
//! идут строго по порядку на одном сервере, а серверы - параллельно, как в Fleet.
//!
//! Три решения здесь приняты в пользу осторожности.
//!
//! **Пробный прогон ничего не меняет.** Он подключается к каждому серверу и проверяет то,
//! что можно проверить без последствий: служба и контейнер существуют, локальные файлы на
//! месте, что именно будет залито. Команды при этом не выполняются вовсе - даже те, что
//! выглядят безобидно: по тексту команды это не определить.
//!
//! **Упавший шаг останавливает задачу на этом сервере**, если у шага не отмечено
//! «продолжать при ошибке». Шаги с условием «если что-то упало» выполняются именно после
//! остановки - это место для отката и уборки.
//!
//! **Хосты с неподтверждённым ключом пропускаются**, как и в Fleet: спрашивать про отпечаток
//! посреди массового прогона некого.

use crate::remote_fs::{self, SessionFs};
use crate::{docker, platform, ssh, store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const DEFAULT_STEP_TIMEOUT_SECS: u64 = 300;
const MAX_STEP_TIMEOUT_SECS: u64 = 6 * 3600;
const MAX_RETRIES: u32 = 10;
const RETRY_DELAY: Duration = Duration::from_secs(3);
const MAX_ATTEMPTS: u32 = 60;
/// Проверки пробного прогона - короткие: они только спрашивают, а не делают.
const PROBE_LIMIT: Duration = Duration::from_secs(20);
/// Вывод шага в окне. Остаётся конец - ошибка обычно там.
const MAX_STEP_OUTPUT: usize = 64 * 1024;
/// Вывод шага в истории: история хранит сотни шагов, и мегабайты в ней не нужны.
const HISTORY_OUTPUT: usize = 8 * 1024;
/// Сколько запусков помнит история.
pub const HISTORY_KEEP: usize = 50;

/// Когда выполнять шаг.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum When {
    /// Если до сих пор всё шло без остановки. Обычный шаг.
    #[default]
    Success,
    /// Только если задача на этом сервере остановилась из-за ошибки: откат, уборка.
    Failure,
    /// В любом случае.
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckKind {
    /// Команда должна завершиться с кодом 0.
    Command,
    /// `адрес:порт` должен принимать соединения - с самого сервера.
    Port,
    /// Адрес должен ответить успешным HTTP - с самого сервера.
    Http,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Action {
    Command {
        command: String,
    },
    /// Залить свой файл или папку в каталог на сервере.
    Upload {
        local_path: String,
        remote_path: String,
    },
    /// Скачать файл или папку с сервера в свою папку. С нескольких серверов - по подпапкам.
    Download {
        remote_path: String,
        local_path: String,
    },
    /// Залить из своей папки только новые и изменённые файлы.
    Sync {
        local_path: String,
        remote_path: String,
        #[serde(default)]
        include_remote_newer: bool,
    },
    Service {
        service: String,
        action: String,
    },
    Docker {
        container: String,
        action: String,
    },
    Healthcheck {
        check: CheckKind,
        target: String,
        #[serde(default)]
        attempts: Option<u32>,
        #[serde(default)]
        interval_sec: Option<u64>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub action: Action,
    #[serde(default)]
    pub when: When,
    #[serde(default)]
    pub continue_on_error: bool,
    #[serde(default)]
    pub retries: u32,
    #[serde(default)]
    pub timeout_sec: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub server_ids: Vec<String>,
    #[serde(default)]
    pub concurrency: Option<u32>,
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(60).collect::<String>())
    } else {
        line.to_owned()
    }
}

impl Step {
    /// Название шага словами: своё, если задано, иначе из того, что шаг делает.
    pub fn label(&self) -> String {
        if let Some(n) = self.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            return n.to_owned();
        }
        match &self.action {
            Action::Command { command } => format!("Команда: {}", first_line(command)),
            Action::Upload { local_path, remote_path } => format!("Залить {local_path} → {remote_path}"),
            Action::Download { remote_path, local_path } => format!("Скачать {remote_path} → {local_path}"),
            Action::Sync { local_path, remote_path, .. } => {
                format!("Синхронизировать {local_path} → {remote_path}")
            }
            Action::Service { service, action } => format!("Служба {service}: {action}"),
            Action::Docker { container, action } => format!("Контейнер {container}: {action}"),
            Action::Healthcheck { check, target, .. } => {
                let what = match check {
                    CheckKind::Command => "командой",
                    CheckKind::Port => "порта",
                    CheckKind::Http => "адреса",
                };
                format!("Проверка {what}: {target}")
            }
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(
            self.timeout_sec
                .filter(|s| *s > 0)
                .unwrap_or(DEFAULT_STEP_TIMEOUT_SECS)
                .min(MAX_STEP_TIMEOUT_SECS),
        )
    }
}

/// Выполнять ли шаг, если до него задача на этом сервере остановилась (`failed`) или нет.
pub fn should_run(when: When, failed: bool) -> bool {
    match when {
        When::Success => !failed,
        When::Failure => failed,
        When::Always => true,
    }
}

/// `адрес:порт`. Адрес - только буквы, цифры и `.-_[]:`: он уходит в команду на сервере.
fn parse_host_port(target: &str) -> Option<(String, u16)> {
    let (host, port) = target.trim().rsplit_once(':')?;
    let port: u16 = port.parse().ok().filter(|p| *p > 0)?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let ok = !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-_:".contains(c));
    ok.then(|| (host.to_owned(), port))
}

/// Адрес для HTTP-проверки: схема обязательна, кавычек и пробелов нет - он уходит в команду.
fn check_url(url: &str) -> Result<(), String> {
    let u = url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err("адрес проверки начинается с http:// или https://".into());
    }
    if u.chars().any(|c| c.is_whitespace() || "'\"`$\\".contains(c)) {
        return Err("в адресе проверки не бывает пробелов, кавычек, `$` и `\\`".into());
    }
    Ok(())
}

/// Имя контейнера - те же символы, что пропускает `docker::action_cmd`. Молча выбрасывать
/// лишнее, как делает он, здесь нельзя: шаг ушёл бы к другому контейнеру.
fn safe_container(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() || !n.chars().all(|c| c.is_alphanumeric() || ".-_".contains(c)) {
        return Err(format!("имя контейнера «{n}»: только буквы, цифры и .-_"));
    }
    Ok(n.to_owned())
}

/// Проверяет задачу целиком до запуска: ошибка в шаге номер семь должна всплыть до того,
/// как шесть шагов уже изменили серверы.
pub fn validate(task: &Task) -> Result<(), String> {
    if task.name.trim().is_empty() {
        return Err("У задачи нет названия".into());
    }
    if task.steps.is_empty() {
        return Err("В задаче нет шагов".into());
    }
    if task.server_ids.is_empty() {
        return Err("Не выбран ни один сервер".into());
    }
    for (i, step) in task.steps.iter().enumerate() {
        let n = i + 1;
        let fail = |what: String| Err(format!("Шаг {n}: {what}"));
        let empty = |v: &str| v.trim().is_empty();
        match &step.action {
            Action::Command { command } if empty(command) => return fail("пустая команда".into()),
            Action::Upload { local_path, remote_path }
            | Action::Download { local_path, remote_path }
            | Action::Sync { local_path, remote_path, .. } => {
                if empty(local_path) || empty(remote_path) {
                    return fail("нужны и своя папка, и путь на сервере".into());
                }
                if let Err(e) = crate::sftp::check_remote_path(remote_path) {
                    return fail(e);
                }
            }
            Action::Service { service, action } => {
                if let Err(e) = crate::workspace::check_service(service, action) {
                    return fail(e);
                }
            }
            Action::Docker { container, action } => {
                if let Err(e) = safe_container(container) {
                    return fail(e);
                }
                if docker::action_cmd(container, action).is_none() {
                    return fail(format!("неизвестное действие с контейнером «{action}»"));
                }
            }
            Action::Healthcheck { check, target, .. } => {
                if empty(target) {
                    return fail("не указано, что проверять".into());
                }
                match check {
                    CheckKind::Port if parse_host_port(target).is_none() => {
                        return fail("порт указывается как адрес:порт, например 127.0.0.1:8080".into())
                    }
                    CheckKind::Http => {
                        if let Err(e) = check_url(target) {
                            return fail(e);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Команда проверки под систему сервера.
fn healthcheck_cmd(kind: platform::Kind, check: CheckKind, target: &str) -> Result<String, String> {
    let windows = kind == platform::Kind::Windows;
    match check {
        CheckKind::Command => Ok(target.to_owned()),
        CheckKind::Port => {
            let (host, port) =
                parse_host_port(target).ok_or("порт указывается как адрес:порт")?;
            if windows {
                return Ok(platform::ps(&format!(
                    "if ((Test-NetConnection -ComputerName '{host}' -Port {port} -WarningAction SilentlyContinue).TcpTestSucceeded) {{ exit 0 }} else {{ exit 1 }}"
                )));
            }
            // `nc` есть не везде, а `/dev/tcp` - только у bash: пробуем по очереди.
            Ok(format!(
                "if command -v nc >/dev/null 2>&1; then nc -z -w 5 {host} {port}; else timeout 5 bash -c 'exec 3<>/dev/tcp/{host}/{port}'; fi"
            ))
        }
        CheckKind::Http => {
            check_url(target)?;
            let url = target.trim();
            if windows {
                return Ok(platform::ps(&format!(
                    "try {{ Invoke-WebRequest -UseBasicParsing -TimeoutSec 10 -Uri '{url}' | Out-Null; exit 0 }} catch {{ exit 1 }}"
                )));
            }
            Ok(format!(
                "if command -v curl >/dev/null 2>&1; then curl -fsS -o /dev/null -m 10 '{url}'; else wget -q -O /dev/null -T 10 '{url}'; fi"
            ))
        }
    }
}

/// Есть ли служба - для пробного прогона.
fn service_exists_cmd(kind: platform::Kind, name: &str) -> Result<String, String> {
    crate::workspace::check_service(name, "restart")?;
    Ok(match kind {
        platform::Kind::Windows => platform::ps(&format!(
            "if (Get-Service -Name '{name}' -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
        )),
        platform::Kind::BusyBox => format!("rc-service -e {name}"),
        _ => format!("systemctl cat -- {name}.service >/dev/null 2>&1"),
    })
}

/// Оставляет конец текста, если он больше предела.
fn tail(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut cut = text.len() - max;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    format!("… (начало обрезано, всего {} Б)\n{}", text.len(), &text[cut..])
}

fn fmt_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} Б")
    } else if n < 1024 * 1024 {
        format!("{:.1} КиБ", n as f64 / 1024.0)
    } else {
        format!("{:.1} МиБ", n as f64 / 1024.0 / 1024.0)
    }
}

/// Имя подпапки под сервер при скачивании с нескольких серверов.
fn safe_dir_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || "-_. ".contains(c) { c } else { '_' })
        .collect();
    let s = s.trim().trim_matches('.').to_owned();
    if s.is_empty() {
        "server".into()
    } else {
        s
    }
}

/// Каталог и имя удалённого пути.
fn split_remote(remote: &str) -> (String, String) {
    let r = remote.trim_end_matches('/');
    match r.rsplit_once('/') {
        Some(("", name)) => ("/".into(), name.into()),
        Some((dir, name)) => (dir.into(), name.into()),
        None => (".".into(), r.into()),
    }
}

/// Всё, что нужно шагу на одном сервере.
struct Host {
    handle: ssh::SharedHandle,
    fs: Arc<Mutex<SessionFs>>,
    kind: platform::Kind,
    server_name: String,
    /// Задача идёт на нескольких серверах: скачанное раскладывается по подпапкам.
    multi: bool,
    alive: Arc<AtomicBool>,
    cancel: ssh::CancelRx,
}

impl Host {
    fn stopped(&self) -> bool {
        *self.cancel.borrow()
    }
}

/// Ждёт, сколько сказано, или до остановки. `true` - остановили.
async fn pause_or_stop(h: &Host, d: Duration) -> bool {
    let mut rx = h.cancel.clone();
    tokio::select! {
        _ = tokio::time::sleep(d) => h.stopped(),
        r = rx.wait_for(|v| *v) => r.is_ok(),
    }
}

async fn exec_ok(h: &Host, cmd: &str, limit: Duration) -> Result<String, String> {
    let (code, out, err) = ssh::exec_timed(&h.handle, cmd, Some(h.cancel.clone()), limit).await?;
    let text = [out.trim_end(), err.trim_end()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if code == 0 {
        Ok(text)
    } else if text.is_empty() {
        Err(format!("код возврата {code}"))
    } else {
        Err(format!("код возврата {code}\n{text}"))
    }
}

/// Создаёт на сервере каталоги, в которые лягут файлы. Существующий каталог - не ошибка.
async fn ensure_remote_parents<'a>(h: &Host, paths: impl Iterator<Item = &'a str>) {
    let mut dirs = BTreeSet::new();
    for p in paths {
        let Some((dir, _)) = p.rsplit_once('/') else { continue };
        let mut cur = String::new();
        for (i, part) in dir.split('/').enumerate() {
            if part.is_empty() {
                if i == 0 {
                    cur.push('/');
                }
                continue;
            }
            if !cur.is_empty() && !cur.ends_with('/') {
                cur.push('/');
            }
            cur.push_str(part);
            dirs.insert(cur.clone());
        }
    }
    // Порядок строк ставит родителя раньше потомка: родитель - начало пути потомка.
    for d in dirs {
        let _ = remote_fs::mkdir(&h.fs, &h.handle, &d).await;
    }
}

async fn put_all(h: &Host, files: &[(String, String, u64)]) -> Result<u64, String> {
    ensure_remote_parents(h, files.iter().map(|(_, rp, _)| rp.as_str())).await;
    let mut bytes = 0;
    for (lp, rp, size) in files {
        if h.stopped() {
            return Err("задача остановлена".into());
        }
        remote_fs::put_file(&h.fs, &h.handle, lp, rp)
            .await
            .map_err(|e| format!("{rp}: {e}"))?;
        bytes += size;
    }
    Ok(bytes)
}

/// Свои файлы для заливки: (свой путь, путь на сервере, размер).
async fn local_files(h: &Host, local: &str, remote_dir: &str) -> Result<Vec<(String, String, u64)>, String> {
    let local = local.replace('\\', "/");
    let root = Path::new(&local)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| format!("не понять имя «{local}»"))?;
    let target = crate::sftp::join_remote(remote_dir, &root);
    let mut files = Vec::new();
    crate::sftp::collect_local(&local, &target, &root, &mut files, Some(h.alive.as_ref())).await?;
    Ok(files.into_iter().map(|(lp, rp, _, size)| (lp, rp, size)).collect())
}

type Jobs = Vec<(String, String, String, u64)>;

/// Что скачивать с сервера и что не будет сохранено. Корень скачивания - `base`.
async fn remote_jobs(h: &Host, remote: &str, base: &str) -> Result<(Jobs, Vec<(String, String)>), String> {
    crate::sftp::check_remote_path(remote)?;
    let (parent, name) = split_remote(remote);
    let listed = remote_fs::list(&h.fs, &h.handle, &parent).await?;
    let entry = listed["entries"]
        .as_array()
        .and_then(|a| a.iter().find(|e| e["name"].as_str() == Some(name.as_str())))
        .ok_or_else(|| format!("на сервере нет «{remote}»"))?
        .clone();
    crate::localname::safe_component(&name).map_err(|why| format!("не сохранить «{name}»: {why}"))?;
    let (jobs, refused) = if entry["type"].as_str() != Some("dir") {
        let size = entry["size"].as_u64().unwrap_or(0);
        (vec![(format!("{base}/{name}"), remote.to_owned(), name.clone(), size)], Vec::new())
    } else if listed["backend"].as_str() == Some("scp") {
        crate::scp::walk_remote(&h.handle, remote, &format!("{base}/{name}"), &name, Some(h.alive.as_ref())).await?
    } else {
        let plan = crate::sftp::plan_download_while(&h.handle, remote, base, Some(h.alive.as_ref())).await?;
        (plan.jobs, plan.refused)
    };
    // Вторая сеть: за пределы папки скачивания не пишем, как бы ни собрался путь.
    if let Some((lp, ..)) = jobs
        .iter()
        .find(|(lp, ..)| !crate::localname::under_root(Path::new(base), Path::new(lp)))
    {
        return Err(format!("путь «{lp}» выходит за пределы папки скачивания"));
    }
    Ok((jobs, refused))
}

fn download_base(h: &Host, local_dir: &str) -> String {
    let dir = local_dir.trim_end_matches(['/', '\\']).replace('\\', "/");
    if h.multi {
        format!("{dir}/{}", safe_dir_name(&h.server_name))
    } else {
        dir
    }
}

async fn sync_targets(
    h: &Host,
    local: &str,
    remote: &str,
    include_remote_newer: bool,
) -> Result<(Vec<(String, String, u64)>, Value), String> {
    let plan = crate::foldersync::compare(&h.fs, &h.handle, local, remote, h.alive.clone()).await?;
    let root = plan["remoteRoot"].as_str().unwrap_or(remote).to_owned();
    let local_root = local.trim_end_matches(['/', '\\']).replace('\\', "/");
    let files = plan["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|it| {
            let k = it["kind"].as_str().unwrap_or("");
            k == "changed" || k == "new" || (include_remote_newer && k == "remoteNewer")
        })
        .filter_map(|it| {
            let rel = it["rel"].as_str()?;
            let size = it["localSize"].as_u64().unwrap_or(0);
            Some((format!("{local_root}/{rel}"), crate::sftp::join_remote(&root, rel), size))
        })
        .collect();
    Ok((files, plan))
}

fn count_kind(plan: &Value, kind: &str) -> usize {
    plan["items"]
        .as_array()
        .map(|a| a.iter().filter(|it| it["kind"].as_str() == Some(kind)).count())
        .unwrap_or(0)
}

async fn run_action(h: &Host, action: &Action, limit: Duration) -> Result<String, String> {
    match action {
        Action::Command { command } => exec_ok(h, command, limit).await,
        Action::Service { service, action } => {
            let cmd = platform::service_cmd(h.kind, service, action)?;
            exec_ok(h, &cmd, limit).await
        }
        Action::Docker { container, action } => {
            let name = safe_container(container)?;
            let cmd = docker::action_cmd(&name, action).ok_or("неизвестное действие с контейнером")?;
            exec_ok(h, &cmd, limit).await
        }
        Action::Upload { local_path, remote_path } => {
            let files = local_files(h, local_path, remote_path).await?;
            let bytes = put_all(h, &files).await?;
            Ok(format!("залито файлов: {} ({})", files.len(), fmt_bytes(bytes)))
        }
        Action::Sync { local_path, remote_path, include_remote_newer } => {
            let (files, plan) = sync_targets(h, local_path, remote_path, *include_remote_newer).await?;
            let bytes = put_all(h, &files).await?;
            Ok(format!(
                "залито файлов: {} ({}); совпадает: {}, на сервере новее: {}",
                files.len(),
                fmt_bytes(bytes),
                count_kind(&plan, "same"),
                count_kind(&plan, "remoteNewer")
            ))
        }
        Action::Download { remote_path, local_path } => {
            let base = download_base(h, local_path);
            let (jobs, refused) = remote_jobs(h, remote_path, &base).await?;
            let mut bytes = 0;
            for (lp, rp, _, size) in &jobs {
                if h.stopped() {
                    return Err("задача остановлена".into());
                }
                if let Some(parent) = Path::new(lp).parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                remote_fs::download_file(&h.fs, &h.handle, rp, lp)
                    .await
                    .map_err(|e| format!("{rp}: {e}"))?;
                bytes += size;
            }
            let mut msg = format!("скачано файлов: {} ({}) в {base}", jobs.len(), fmt_bytes(bytes));
            if !refused.is_empty() {
                let list: Vec<String> = refused.iter().map(|(r, why)| format!("{r} - {why}")).collect();
                msg.push_str(&format!("\nне сохранено: {}", list.join("; ")));
            }
            Ok(msg)
        }
        Action::Healthcheck { check, target, attempts, interval_sec } => {
            let cmd = healthcheck_cmd(h.kind, *check, target)?;
            let attempts = attempts.unwrap_or(3).clamp(1, MAX_ATTEMPTS);
            let interval = Duration::from_secs(interval_sec.unwrap_or(5).clamp(1, 600));
            let per_try = limit.min(Duration::from_secs(60));
            let mut last = String::new();
            for n in 1..=attempts {
                match exec_ok(h, &cmd, per_try).await {
                    Ok(_) => return Ok(format!("ответил с попытки {n} из {attempts}")),
                    Err(e) => last = e,
                }
                if n < attempts && pause_or_stop(h, interval).await {
                    return Err("задача остановлена".into());
                }
            }
            Err(format!("не ответил за {attempts} попыток: {last}"))
        }
    }
}

/// Пробный прогон шага: что будет сделано и можно ли это сделать. Ничего не меняет.
async fn plan_action(h: &Host, action: &Action) -> Result<String, String> {
    match action {
        Action::Command { command } => Ok(format!("выполнится: {}", first_line(command))),
        Action::Service { service, action } => {
            platform::service_cmd(h.kind, service, action)?;
            let (code, ..) =
                ssh::exec_timed(&h.handle, &service_exists_cmd(h.kind, service)?, Some(h.cancel.clone()), PROBE_LIMIT)
                    .await?;
            if code == 0 {
                Ok(format!("служба {service} есть, действие: {action}"))
            } else {
                Err(format!("службы «{service}» на сервере нет"))
            }
        }
        Action::Docker { container, action } => {
            let name = safe_container(container)?;
            let cmd = format!("docker inspect --format '{{{{.State.Status}}}}' {name}");
            let (code, out, err) = ssh::exec_timed(&h.handle, &cmd, Some(h.cancel.clone()), PROBE_LIMIT).await?;
            if code == 0 {
                Ok(format!("контейнер {name}: {}, действие: {action}", out.trim()))
            } else if err.trim().is_empty() {
                Err(format!("контейнера «{name}» нет"))
            } else {
                Err(err.trim().to_owned())
            }
        }
        Action::Upload { local_path, remote_path } => {
            let files = local_files(h, local_path, remote_path).await?;
            let bytes: u64 = files.iter().map(|(.., s)| s).sum();
            let exists = remote_fs::list(&h.fs, &h.handle, remote_path).await.is_ok();
            Ok(format!(
                "будет залито файлов: {} ({}) в {remote_path}{}",
                files.len(),
                fmt_bytes(bytes),
                if exists { "" } else { " - каталога нет, он будет создан" }
            ))
        }
        Action::Sync { local_path, remote_path, include_remote_newer } => {
            let (files, plan) = sync_targets(h, local_path, remote_path, *include_remote_newer).await?;
            let bytes: u64 = files.iter().map(|(.., s)| s).sum();
            Ok(format!(
                "будет залито файлов: {} ({}): изменено {}, новых {}{}; совпадает {}",
                files.len(),
                fmt_bytes(bytes),
                count_kind(&plan, "changed"),
                count_kind(&plan, "new"),
                if *include_remote_newer {
                    format!(", на сервере новее {}", count_kind(&plan, "remoteNewer"))
                } else {
                    String::new()
                },
                count_kind(&plan, "same")
            ))
        }
        Action::Download { remote_path, local_path } => {
            let base = download_base(h, local_path);
            let (jobs, refused) = remote_jobs(h, remote_path, &base).await?;
            let bytes: u64 = jobs.iter().map(|(.., s)| s).sum();
            let mut msg = format!("будет скачано файлов: {} ({}) в {base}", jobs.len(), fmt_bytes(bytes));
            if !refused.is_empty() {
                msg.push_str(&format!("; не будет сохранено: {}", refused.len()));
            }
            Ok(msg)
        }
        Action::Healthcheck { check, target, attempts, .. } => {
            healthcheck_cmd(h.kind, *check, target)?;
            Ok(format!("проверит {target}, попыток: {}", attempts.unwrap_or(3).clamp(1, MAX_ATTEMPTS)))
        }
    }
}

/// Шаг с повторами и сроком. `(успех, вывод, сколько попыток ушло)`.
async fn run_step(h: &Host, step: &Step) -> (bool, String, u32) {
    let attempts = 1 + step.retries.min(MAX_RETRIES);
    let limit = step.timeout();
    let mut last = String::new();
    for n in 1..=attempts {
        if h.stopped() {
            return (false, "задача остановлена".into(), n - 1);
        }
        match tokio::time::timeout(limit, run_action(h, &step.action, limit)).await {
            Ok(Ok(out)) => return (true, tail(out, MAX_STEP_OUTPUT), n),
            Ok(Err(e)) => last = e,
            Err(_) => last = format!("шаг не уложился в {} с", limit.as_secs()),
        }
        if n < attempts && pause_or_stop(h, RETRY_DELAY).await {
            return (false, "задача остановлена".into(), n);
        }
    }
    (false, tail(last, MAX_STEP_OUTPUT), attempts)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct RunCtx<'a> {
    task: &'a Task,
    run_id: &'a str,
    dry_run: bool,
    multi: bool,
    cancel: ssh::CancelRx,
    emit: &'a (dyn Fn(Value) + Sync),
}

fn step_json(i: usize, step: &Step, state: &str, output: &str, ms: u128, attempts: u32) -> Value {
    json!({
        "index": i,
        "label": step.label(),
        "state": state,
        "output": output,
        "ms": ms,
        "attempts": attempts,
    })
}

async fn run_server(ctx: &RunCtx<'_>, server_id: String) -> Value {
    let started = Instant::now();
    let name = crate::multihost::name_of(&server_id);
    let emit = |step: Option<usize>, state: &str, output: Option<&str>| {
        (ctx.emit)(json!({
            "runId": ctx.run_id,
            "serverId": server_id,
            "name": name,
            "step": step,
            "state": state,
            "output": output,
        }))
    };
    let finish = |state: &str, error: Option<String>, steps: Vec<Value>| {
        emit(None, state, error.as_deref());
        json!({
            "serverId": server_id,
            "name": name,
            "state": state,
            "error": error,
            "steps": steps,
            "ms": started.elapsed().as_millis(),
        })
    };

    let chain = match crate::resolve_chain_for(&server_id) {
        Ok(c) => c,
        Err(e) => return finish("skipped", Some(e), Vec::new()),
    };
    if chain.first().and_then(|s| s.get("connection")).and_then(|v| v.as_str()) == Some("serial") {
        return finish("skipped", Some("COM-порт: задачи не поддерживаются".into()), Vec::new());
    }
    if let Some(why) = crate::multihost::skip_reason(&chain) {
        return finish("skipped", Some(why), Vec::new());
    }
    if *ctx.cancel.borrow() {
        return finish("cancelled", Some("задача остановлена до подключения".into()), Vec::new());
    }
    emit(None, "connecting", None);
    let handle = match ssh::connect_client(chain).await {
        Ok(h) => h,
        Err(e) => return finish("failed", Some(e.to_string()), Vec::new()),
    };
    let key = format!("task:{}:{server_id}", ctx.run_id);
    let (kind, _) = platform::of_session(&key, &handle).await;
    let host = Host {
        handle,
        fs: Arc::new(Mutex::new(SessionFs::new())),
        kind,
        server_name: name.clone(),
        multi: ctx.multi,
        alive: Arc::new(AtomicBool::new(true)),
        cancel: ctx.cancel.clone(),
    };

    let mut failed = false;
    let mut problems = 0usize;
    let mut steps = Vec::with_capacity(ctx.task.steps.len());
    for (i, step) in ctx.task.steps.iter().enumerate() {
        let t = Instant::now();
        if host.stopped() {
            emit(Some(i), "cancelled", None);
            steps.push(step_json(i, step, "cancelled", "задача остановлена", 0, 0));
            continue;
        }
        if ctx.dry_run {
            emit(Some(i), "running", None);
            let (state, mut out) = match plan_action(&host, &step.action).await {
                Ok(text) => ("planned", text),
                Err(e) => {
                    problems += 1;
                    ("problem", e)
                }
            };
            match step.when {
                When::Failure => out.push_str(" (только если задача на сервере остановится из-за ошибки)"),
                When::Always => out.push_str(" (в любом случае)"),
                When::Success => {}
            }
            emit(Some(i), state, Some(&out));
            steps.push(step_json(i, step, state, &out, t.elapsed().as_millis(), 0));
            continue;
        }
        if !should_run(step.when, failed) {
            let why = if failed {
                "не выполнялся: задача остановилась на ошибке"
            } else {
                "не выполнялся: ошибок не было"
            };
            emit(Some(i), "skipped", Some(why));
            steps.push(step_json(i, step, "skipped", why, 0, 0));
            continue;
        }
        emit(Some(i), "running", None);
        let (ok, out, attempts) = run_step(&host, step).await;
        let state = if ok { "done" } else { "failed" };
        emit(Some(i), state, Some(&out));
        steps.push(step_json(i, step, state, &out, t.elapsed().as_millis(), attempts));
        if !ok {
            problems += 1;
            if !step.continue_on_error {
                failed = true;
            }
        }
    }
    host.alive.store(false, Ordering::Relaxed);
    platform::forget(&key);

    let state = if host.stopped() {
        "cancelled"
    } else if failed {
        "failed"
    } else if ctx.dry_run && problems > 0 {
        "problem"
    } else {
        "done"
    };
    let mut out = finish(state, None, steps);
    out["errors"] = json!(problems);
    out
}

/// Копия отчёта для истории: выводы шагов короче.
fn for_history(mut report: Value) -> Value {
    if let Some(servers) = report["servers"].as_array_mut() {
        for s in servers {
            if let Some(steps) = s["steps"].as_array_mut() {
                for st in steps {
                    if let Some(text) = st["output"].as_str().map(str::to_owned) {
                        st["output"] = json!(tail(text, HISTORY_OUTPUT));
                    }
                }
            }
        }
    }
    report
}

/// Прогон задачи. Ход каждого шага уходит событием `task-progress`; в конце - отчёт.
pub async fn run(
    app: AppHandle,
    task: Task,
    run_id: String,
    dry_run: bool,
    cancel: ssh::CancelRx,
) -> Result<Value, String> {
    let emit = move |event: Value| {
        let _ = app.emit("task-progress", event);
    };
    let report = run_with(&emit, &task, &run_id, dry_run, cancel).await?;
    // Пробный прогон в историю не пишем: история - это что с серверами делали на самом деле.
    if !dry_run {
        if let Err(e) = store::task_runs_add(for_history(report.clone()), HISTORY_KEEP) {
            crate::rdp::log(&format!("история задач не записана: {e}"));
        }
    }
    Ok(report)
}

async fn run_with(
    emit: &(dyn Fn(Value) + Sync),
    task: &Task,
    run_id: &str,
    dry_run: bool,
    cancel: ssh::CancelRx,
) -> Result<Value, String> {
    use futures::stream::{FuturesUnordered, StreamExt};

    validate(task)?;
    let started_at = now_ms();
    let ctx = RunCtx {
        task,
        run_id,
        dry_run,
        multi: task.server_ids.len() > 1,
        cancel: cancel.clone(),
        emit,
    };
    let concurrency = task.concurrency.unwrap_or(4).clamp(1, 64) as usize;
    let mut queue = task.server_ids.clone().into_iter();
    let mut running = FuturesUnordered::new();
    let mut servers = Vec::with_capacity(task.server_ids.len());
    for _ in 0..concurrency {
        match queue.next() {
            Some(id) => running.push(run_server(&ctx, id)),
            None => break,
        }
    }
    while let Some(res) = running.next().await {
        servers.push(res);
        if let Some(id) = queue.next() {
            // После остановки run_server сам завершится без подключения.
            running.push(run_server(&ctx, id));
        }
    }
    Ok(json!({
        "runId": run_id,
        "taskId": task.id,
        "taskName": task.name,
        "dryRun": dry_run,
        "cancelled": *cancel.borrow(),
        "startedAt": started_at,
        "finishedAt": now_ms(),
        "servers": servers,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_json() -> Value {
        json!({
            "id": "t1",
            "name": "Выкладка",
            "serverIds": ["a", "b"],
            "steps": [
                { "id": "s1", "kind": "upload", "localPath": "C:/build/site", "remotePath": "/var/www" },
                { "kind": "service", "service": "nginx", "action": "restart", "retries": 2 },
                { "kind": "healthcheck", "check": "http", "target": "http://127.0.0.1/", "attempts": 5, "intervalSec": 2 },
                { "kind": "command", "command": "systemctl rollback-site", "when": "failure", "continueOnError": true },
            ]
        })
    }

    #[test]
    fn задача_из_окна_разбирается_с_умолчаниями() {
        let t: Task = serde_json::from_value(task_json()).expect("разбор");
        assert_eq!(t.steps.len(), 4);
        assert_eq!(t.steps[0].when, When::Success, "по умолчанию шаг идёт после успешных");
        assert_eq!(t.steps[1].retries, 2);
        assert_eq!(t.steps[3].when, When::Failure);
        assert!(t.steps[3].continue_on_error);
        assert!(matches!(
            &t.steps[2].action,
            Action::Healthcheck { check: CheckKind::Http, attempts: Some(5), interval_sec: Some(2), .. }
        ));
        assert_eq!(t.steps[0].label(), "Залить C:/build/site → /var/www");
        validate(&t).expect("задача годная");
    }

    #[test]
    fn проверка_задачи_находит_ошибку_до_запуска() {
        let mut t: Task = serde_json::from_value(task_json()).unwrap();
        t.steps.push(serde_json::from_value(json!({ "kind": "docker", "container": "web", "action": "explode" })).unwrap());
        assert!(validate(&t).unwrap_err().starts_with("Шаг 5:"), "номер шага в тексте");

        let mut t: Task = serde_json::from_value(task_json()).unwrap();
        t.steps[0] = serde_json::from_value(json!({ "kind": "upload", "localPath": "x", "remotePath": "/var/../etc" })).unwrap();
        assert!(validate(&t).is_err(), "«..» в пути на сервере");

        let mut t: Task = serde_json::from_value(task_json()).unwrap();
        t.steps[2] = serde_json::from_value(json!({ "kind": "healthcheck", "check": "port", "target": "8080" })).unwrap();
        assert!(validate(&t).unwrap_err().contains("адрес:порт"));

        let mut t: Task = serde_json::from_value(task_json()).unwrap();
        t.server_ids.clear();
        assert_eq!(validate(&t).unwrap_err(), "Не выбран ни один сервер");
    }

    #[test]
    fn условия_шагов() {
        assert!(should_run(When::Success, false));
        assert!(!should_run(When::Success, true), "после остановки обычные шаги не идут");
        assert!(should_run(When::Failure, true), "откат - именно после остановки");
        assert!(!should_run(When::Failure, false));
        assert!(should_run(When::Always, true) && should_run(When::Always, false));
    }

    #[test]
    fn команды_проверок_под_систему() {
        let port = healthcheck_cmd(platform::Kind::Linux, CheckKind::Port, "127.0.0.1:8080").unwrap();
        assert!(port.contains("nc -z -w 5 127.0.0.1 8080"), "{port}");
        let http = healthcheck_cmd(platform::Kind::Linux, CheckKind::Http, "https://example.org/health").unwrap();
        assert!(http.contains("curl -fsS -o /dev/null -m 10 'https://example.org/health'"), "{http}");
        assert!(healthcheck_cmd(platform::Kind::Linux, CheckKind::Http, "https://x/'; rm -rf /").is_err());
        assert!(healthcheck_cmd(platform::Kind::Linux, CheckKind::Port, "host;rm:22").is_err());
        let win = healthcheck_cmd(platform::Kind::Windows, CheckKind::Port, "db:1433").unwrap();
        assert!(win.to_lowercase().contains("powershell"), "{win}");
        assert_eq!(parse_host_port("[::1]:443"), Some(("::1".to_owned(), 443)));
    }

    #[test]
    fn пути_и_имена() {
        assert_eq!(split_remote("/var/log/nginx/"), ("/var/log".to_owned(), "nginx".to_owned()));
        assert_eq!(split_remote("/etc"), ("/".to_owned(), "etc".to_owned()));
        assert_eq!(split_remote("site"), (".".to_owned(), "site".to_owned()));
        assert_eq!(safe_dir_name("prod/db: основной"), "prod_db_ основной");
        assert_eq!(safe_dir_name("../.."), "_", "точки по краям снимаются - выше папки не уйти");
        assert!(safe_container("web;rm").is_err());
    }

    #[test]
    fn история_хранит_конец_вывода() {
        let long = format!("{}итог", "x".repeat(HISTORY_OUTPUT * 2));
        let report = json!({ "servers": [ { "steps": [ { "output": long } ] } ] });
        let short = for_history(report);
        let out = short["servers"][0]["steps"][0]["output"].as_str().unwrap();
        assert!(out.len() < HISTORY_OUTPUT + 100);
        assert!(out.ends_with("итог"));
    }

    #[test]
    fn остановленная_задача_не_подключается() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut t: Task = serde_json::from_value(task_json()).unwrap();
        t.server_ids = vec!["нет-такого-1".into(), "нет-такого-2".into()];
        let (_tx, rx) = tokio::sync::watch::channel(true);
        let events = Mutex::new(0usize);
        let emit = |_: Value| *events.lock().unwrap() += 1;
        let report = rt.block_on(run_with(&emit, &t, "run-1", false, rx)).expect("отчёт");
        let servers = report["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 2);
        for s in servers {
            assert_ne!(s["state"], "done", "{s}");
            assert_eq!(s["steps"].as_array().map(|a| a.len()), Some(0), "шаги не начинались: {s}");
        }
        assert!(*events.lock().unwrap() >= 2, "окно узнало о каждом сервере");
        assert_eq!(report["cancelled"], true);
    }
}
