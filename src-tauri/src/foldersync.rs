//! Сравнение локальной и удалённой папки - узкий срез синхронизации.
//!
//! Не замена rsync. Три вещи, которые экономят действия: увидеть разницу, залить только
//! изменённое и знать заранее, что будет сделано. Удаления лишнего здесь нет вовсе, как и
//! хешей: сравнение по размеру и времени правки, а сама заливка идёт обычной очередью
//! передач - с её прогрессом, отменой и записью через временный файл.

use crate::remote_fs::{self, SessionFs};
use crate::sftp::{join_remote, walk_too_big, MAX_WALK_DEPTH, MAX_WALK_ENTRIES};
use crate::ssh::SharedHandle;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Допуск на время правки. Часы машины и сервера расходятся, а часть файловых систем хранит
/// время с точностью до двух секунд (FAT и exFAT на флешках).
const MTIME_TOLERANCE_MS: u64 = 2000;

/// Что делать с файлом. Порядок - порядок показа: сначала то, что будет залито.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Есть с обеих сторон, локальный отличается и не старше серверного.
    Changed,
    /// Только локально.
    New,
    /// На сервере правили позже - по умолчанию не заливаем, чтобы не затереть чужое.
    RemoteNewer,
    /// Размер тот же, а времени правки хотя бы с одной стороны нет: одинаковое содержимое
    /// этим не доказано. По умолчанию не заливаем, но и «совпадает» не пишем.
    Unsure,
    /// Только на сервере. Удалять не предлагаем.
    RemoteOnly,
    Same,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Changed => "changed",
            Kind::New => "new",
            Kind::RemoteNewer => "remoteNewer",
            Kind::Unsure => "unsure",
            Kind::RemoteOnly => "remoteOnly",
            Kind::Same => "same",
        }
    }
}

/// Размер и время правки в миллисекундах. Время 0 - неизвестно.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp {
    pub size: u64,
    pub mtime: u64,
}

/// Решение по одному файлу. `time_known` - знаем ли время правки на сервере: по SCP его
/// нет, и тогда одинаковый размер - «не определить», а не «совпадает».
pub fn classify(local: Option<Stamp>, remote: Option<Stamp>, time_known: bool) -> Kind {
    let (l, r) = match (local, remote) {
        (Some(_), None) => return Kind::New,
        (Some(l), Some(r)) => (l, r),
        _ => return Kind::RemoteOnly,
    };
    if !time_known || r.mtime == 0 || l.mtime == 0 {
        // Одинаковый размер без времени правки ничего не доказывает: «mode=dev» и «mode=prd»
        // одной длины.
        return if l.size == r.size { Kind::Unsure } else { Kind::Changed };
    }
    if l.mtime > r.mtime + MTIME_TOLERANCE_MS {
        return Kind::Changed;
    }
    if r.mtime > l.mtime + MTIME_TOLERANCE_MS {
        // Правили на сервере позже - и неважно, сменился ли размер. Своя заливка сюда не
        // попадает: она переносит на сервер время правки своего файла.
        return Kind::RemoteNewer;
    }
    if l.size == r.size {
        Kind::Same
    } else {
        Kind::Changed
    }
}

/// Изменился ли файл на сервере после сравнения - проверка прямо перед заливкой.
///
/// `seen` - время правки из плана (`None` - файла тогда не было, `Some(0)` - время не
/// узнали), `now` - ответ сервера сейчас. Пропавший или непроверяемый сейчас файл, который
/// при сравнении был, - тоже «изменился»: заливать вслепую поверх нельзя.
pub fn moved_since_plan(seen: Option<u64>, now: &Result<Option<u64>, String>) -> bool {
    match (seen, now) {
        (None, Ok(Some(_))) => true,
        (None, _) => false,
        (Some(0), _) => false,
        (Some(seen), Ok(Some(now))) => *now != seen,
        (Some(_), _) => true,
    }
}

/// Одна сторона сравнения: файлы, каталоги и то, что сравнивать не стали.
#[derive(Default, Debug)]
struct Side {
    files: BTreeMap<String, Stamp>,
    dirs: BTreeSet<String>,
    refused: Vec<(String, String)>,
}

fn join_rel(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_owned()
    } else {
        format!("{rel}/{name}")
    }
}

/// Обход своей папки. Пределы и правило ссылок - те же, что у передачи папок.
fn walk_local(root: &Path, alive: &AtomicBool) -> Result<Side, String> {
    let mut side = Side::default();
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 0)];
    while let Some((dir, rel, depth)) = stack.pop() {
        if !alive.load(Ordering::Relaxed) {
            return Err("Сессия закрыта".into());
        }
        if depth > MAX_WALK_DEPTH {
            return Err(walk_too_big());
        }
        let rd = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for ent in rd {
            let ent = ent.map_err(|e| e.to_string())?;
            let name = ent.file_name().to_string_lossy().to_string();
            let r = join_rel(&rel, &name);
            let path = ent.path();
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    side.refused.push((r, e.to_string()));
                    continue;
                }
            };
            let meta = if meta.file_type().is_symlink() {
                match std::fs::metadata(&path) {
                    Ok(t) if t.is_dir() => {
                        side.refused.push((r, "ссылка на каталог - внутрь не заходим".into()));
                        continue;
                    }
                    Ok(t) => t,
                    Err(_) => {
                        side.refused.push((r, "ссылка никуда не ведёт".into()));
                        continue;
                    }
                }
            } else {
                meta
            };
            if meta.is_dir() {
                side.dirs.insert(r.clone());
                stack.push((path, r, depth + 1));
            } else if meta.is_file() {
                if side.files.len() >= MAX_WALK_ENTRIES {
                    return Err(walk_too_big());
                }
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                side.files.insert(r, Stamp { size: meta.len(), mtime });
            }
        }
    }
    Ok(side)
}

/// Обход удалённой папки тем же листингом, что и панель: SFTP или SCP - как выбрала сессия.
/// Вторым - знаем ли время правки, третьим - абсолютный путь корня.
async fn walk_remote(
    fs: &Arc<Mutex<SessionFs>>,
    handle: &SharedHandle,
    root: &str,
    alive: &AtomicBool,
) -> Result<(Side, bool, String), String> {
    let first = remote_fs::list(fs, handle, root).await?;
    let abs = first["path"].as_str().unwrap_or(root).to_owned();
    // `ls` через SCP время правки не отдаёт - сравнение тогда только по размеру.
    let time_known = first["backend"].as_str() != Some("scp");
    let mut side = Side::default();
    let mut pending: Vec<(String, String, usize, Option<Value>)> = vec![(abs.clone(), String::new(), 0, Some(first))];
    while let Some((dir, rel, depth, listed)) = pending.pop() {
        if !alive.load(Ordering::Relaxed) {
            return Err("Сессия закрыта".into());
        }
        if depth > MAX_WALK_DEPTH {
            return Err(walk_too_big());
        }
        let listed = match listed {
            Some(v) => v,
            None => remote_fs::list(fs, handle, &dir).await?,
        };
        for e in listed["entries"].as_array().into_iter().flatten() {
            let name = e["name"].as_str().unwrap_or("");
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            let r = join_rel(&rel, name);
            match e["type"].as_str() {
                Some("dir") => {
                    side.dirs.insert(r.clone());
                    pending.push((join_remote(&dir, name), r, depth + 1, None));
                }
                Some("file") => {
                    if side.files.len() >= MAX_WALK_ENTRIES {
                        return Err(walk_too_big());
                    }
                    let stamp = Stamp {
                        size: e["size"].as_u64().unwrap_or(0),
                        mtime: e["mtime"].as_u64().unwrap_or(0),
                    };
                    side.files.insert(r, stamp);
                }
                Some("link") => side.refused.push((r, "ссылка на сервере - не сравниваем".into())),
                _ => {}
            }
        }
    }
    Ok((side, time_known, abs))
}

/// План: что с каждым файлом и что сравнивать не стали.
fn plan(local: &Side, remote: &Side, time_known: bool, local_root: &str, remote_root: &str) -> Value {
    let mut refused: Vec<Value> = Vec::new();
    for (rel, why) in &local.refused {
        refused.push(json!({ "rel": rel, "why": format!("локально: {why}") }));
    }
    for (rel, why) in &remote.refused {
        refused.push(json!({ "rel": rel, "why": why }));
    }

    // Имя, которое с одной стороны файл, а с другой каталог, заливкой не решается: поверх
    // каталога файл не ляжет, а в файл не положить содержимое папки. Такое называем словами
    // и всё, что под ним, не трогаем.
    let mut blocked: Vec<String> = Vec::new();
    for rel in local.files.keys().filter(|r| remote.dirs.contains(*r)) {
        refused.push(json!({ "rel": rel, "why": "на сервере с этим именем каталог" }));
        blocked.push(rel.clone());
    }
    for rel in local.dirs.iter().filter(|r| remote.files.contains_key(*r)) {
        refused.push(json!({ "rel": rel, "why": "на сервере с этим именем файл - содержимое папки не залить" }));
        blocked.push(rel.clone());
    }
    let is_blocked =
        |rel: &str| blocked.iter().any(|b| rel == b || rel.starts_with(&format!("{b}/")));

    let names: BTreeSet<&String> = local.files.keys().chain(remote.files.keys()).collect();
    let mut rows: Vec<(Kind, &String, Option<Stamp>, Option<Stamp>)> = names
        .into_iter()
        .filter(|rel| !is_blocked(rel))
        .map(|rel| {
            let l = local.files.get(rel).copied();
            let r = remote.files.get(rel).copied();
            (classify(l, r, time_known), rel, l, r)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(kind, rel, l, r)| {
            json!({
                "rel": rel,
                "kind": kind.as_str(),
                "localSize": l.map(|s| s.size),
                "remoteSize": r.map(|s| s.size),
                "localMtime": l.map(|s| s.mtime),
                "remoteMtime": r.map(|s| s.mtime),
            })
        })
        .collect();
    json!({
        "localRoot": local_root,
        "remoteRoot": remote_root,
        "timeKnown": time_known,
        "items": items,
        "remoteDirs": remote.dirs,
        "refused": refused,
    })
}

/// Сравнивает локальную папку с удалённой. Ничего не пишет ни с одной стороны.
pub async fn compare(
    fs: &Arc<Mutex<SessionFs>>,
    handle: &SharedHandle,
    local_dir: &str,
    remote_dir: &str,
    alive: Arc<AtomicBool>,
) -> Result<Value, String> {
    crate::sftp::check_remote_path(remote_dir)?;
    let root = PathBuf::from(local_dir);
    if !root.is_dir() {
        return Err(format!("Локальной папки «{local_dir}» нет"));
    }
    let alive_local = alive.clone();
    let local = tokio::task::spawn_blocking(move || walk_local(&root, &alive_local))
        .await
        .map_err(|e| format!("Обход папки прерван: {e}"))??;
    let (remote, time_known, remote_abs) = walk_remote(fs, handle, remote_dir, &alive).await?;
    Ok(plan(&local, &remote, time_known, local_dir, &remote_abs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(size: u64, mtime: u64) -> Option<Stamp> {
        Some(Stamp { size, mtime })
    }

    #[test]
    fn решение_по_размеру_и_времени_правки() {
        let t = 1_000_000;
        assert_eq!(classify(st(5, t), None, true), Kind::New);
        assert_eq!(classify(None, st(5, t), true), Kind::RemoteOnly);
        assert_eq!(classify(st(5, t), st(5, t + 1500), true), Kind::Same, "в пределах допуска");
        assert_eq!(classify(st(5, t + 60_000), st(5, t), true), Kind::Changed, "правили локально");
        assert_eq!(classify(st(6, t), st(5, t), true), Kind::Changed, "размер другой, время то же");
        assert_eq!(classify(st(6, t), st(5, t + 60_000), true), Kind::RemoteNewer, "правили на сервере");
        // Размер тот же, но на сервере правили позже: содержимое могло смениться.
        assert_eq!(classify(st(5, t), st(5, t + 60_000), true), Kind::RemoteNewer);
    }

    #[test]
    fn без_времени_правки_одинаковый_размер_не_значит_совпадает() {
        let t = 1_000_000;
        assert_eq!(classify(st(5, t + 60_000), st(5, 0), false), Kind::Unsure);
        assert_eq!(classify(st(5, 0), st(5, t), true), Kind::Unsure, "своё время неизвестно");
        assert_eq!(classify(st(6, t), st(5, 0), false), Kind::Changed);
    }

    #[test]
    fn правка_на_сервере_после_сравнения_не_затирается() {
        let ok: Result<Option<u64>, String> = Ok(Some(5_000));
        assert!(!moved_since_plan(Some(5_000), &ok), "не менялся");
        assert!(moved_since_plan(Some(4_000), &ok), "время другое");
        assert!(moved_since_plan(Some(5_000), &Err("нет файла".into())), "пропал или не проверить");
        assert!(moved_since_plan(Some(5_000), &Ok(None)), "не проверить");
        assert!(moved_since_plan(None, &ok), "появился, пока ждали");
        assert!(!moved_since_plan(None, &Err("нет файла".into())));
        assert!(!moved_since_plan(Some(0), &ok), "времени в плане не было - сравнить не с чем");
    }

    #[test]
    fn план_не_заливает_файл_поверх_каталога() {
        let mut local = Side::default();
        local.files.insert("a".into(), Stamp { size: 1, mtime: 1 });
        local.files.insert("b/x.txt".into(), Stamp { size: 1, mtime: 1 });
        local.dirs.insert("b".into());
        local.files.insert("новый.txt".into(), Stamp { size: 3, mtime: 1 });
        let mut remote = Side::default();
        remote.dirs.insert("a".into());
        remote.files.insert("b".into(), Stamp { size: 1, mtime: 1 });
        remote.dirs.insert("есть".into());
        let v = plan(&local, &remote, true, "C:/local", "/srv");
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "a, b и b/x.txt отложены: {v}");
        assert_eq!(items[0]["rel"], "новый.txt");
        assert_eq!(items[0]["kind"], "new");
        let why: Vec<&str> = v["refused"].as_array().unwrap().iter().map(|r| r["rel"].as_str().unwrap()).collect();
        assert!(why.contains(&"a") && why.contains(&"b"), "{why:?}");
        assert_eq!(v["remoteDirs"], json!(["a", "есть"]), "существующие каталоги сервера - все");
    }

    #[test]
    fn обход_своей_папки_собирает_вложенное_и_останавливается_с_сессией() {
        let dir = std::env::temp_dir().join(format!("serein-sync-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("вложенная")).unwrap();
        std::fs::write(dir.join("a.txt"), "12345").unwrap();
        std::fs::write(dir.join("вложенная").join("b.txt"), "1").unwrap();

        let side = walk_local(&dir, &AtomicBool::new(true)).expect("обход");
        assert_eq!(side.files.get("a.txt").map(|s| s.size), Some(5));
        assert!(side.files.contains_key("вложенная/b.txt"), "{side:?}");
        assert!(side.dirs.contains("вложенная"));

        let err = walk_local(&dir, &AtomicBool::new(false)).unwrap_err();
        assert_eq!(err, "Сессия закрыта");
        let _ = std::fs::remove_dir_all(dir);
    }
}
