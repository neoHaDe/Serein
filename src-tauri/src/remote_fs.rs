//! Файловый менеджер: SFTP, при недоступности подсистемы - SCP + ls/exec.

use crate::scp;
use crate::sftp::{self, TransferHub};
use crate::ssh::SharedHandle;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Sftp,
    Scp,
}

impl Backend {
    fn as_str(self) -> &'static str {
        match self {
            Backend::Sftp => "sftp",
            Backend::Scp => "scp",
        }
    }
}

pub struct SessionFs {
    backend: Option<Backend>,
}

impl Default for SessionFs {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionFs {
    pub fn new() -> Self {
        Self { backend: None }
    }

    pub async fn resolve(&mut self, handle: &SharedHandle) -> Backend {
        if let Some(b) = self.backend {
            return b;
        }
        let b = if sftp::probe(handle).await {
            Backend::Sftp
        } else {
            Backend::Scp
        };
        self.backend = Some(b);
        b
    }

    pub fn known(&self) -> Option<Backend> {
        self.backend
    }
}

async fn backend(fs: &Arc<Mutex<SessionFs>>, handle: &SharedHandle) -> Backend {
    {
        let g = crate::sync::lock(fs);
        if let Some(b) = g.backend {
            return b;
        }
    }
    let b = if sftp::probe(handle).await {
        Backend::Sftp
    } else {
        Backend::Scp
    };
    crate::sync::lock(fs).backend = Some(b);
    b
}

fn tag(mut v: Value, b: Backend) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.insert("backend".into(), json!(b.as_str()));
    }
    v
}

pub async fn list(fs: &Arc<Mutex<SessionFs>>, handle: &SharedHandle, path: &str) -> Result<Value, String> {
    let b = backend(fs, handle).await;
    match b {
        Backend::Sftp => Ok(tag(sftp::list(handle, path).await?, b)),
        Backend::Scp => scp::list(handle, path).await,
    }
}

pub async fn mkdir(fs: &Arc<Mutex<SessionFs>>, handle: &SharedHandle, path: &str) -> Result<(), String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::mkdir(handle, path).await,
        Backend::Scp => scp::mkdir(handle, path).await,
    }
}

pub async fn remove(fs: &Arc<Mutex<SessionFs>>, handle: &SharedHandle, path: &str, is_dir: bool) -> Result<(), String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::remove(handle, path, is_dir).await,
        Backend::Scp => scp::remove(handle, path, is_dir).await,
    }
}

pub async fn rename(fs: &Arc<Mutex<SessionFs>>, handle: &SharedHandle, from: &str, to: &str) -> Result<(), String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::rename(handle, from, to).await,
        Backend::Scp => scp::rename(handle, from, to).await,
    }
}

pub async fn chmod(fs: &Arc<Mutex<SessionFs>>, handle: &SharedHandle, path: &str, mode: u32) -> Result<(), String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::chmod(handle, path, mode).await,
        Backend::Scp => scp::chmod(handle, path, mode).await,
    }
}

pub async fn preview(fs: &Arc<Mutex<SessionFs>>, handle: &SharedHandle, remote: &str) -> Result<Value, String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::preview(handle, remote).await,
        Backend::Scp => scp::preview(handle, remote).await,
    }
}

/// Когда файл на сервере правили последний раз. `None` - не узнать этим способом.
pub async fn remote_mtime(
    fs: &Arc<Mutex<SessionFs>>,
    handle: &SharedHandle,
    remote: &str,
) -> Result<Option<u64>, String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::remote_mtime(handle, remote).await,
        Backend::Scp => scp::remote_mtime(handle, remote).await,
    }
}

pub async fn read_file(fs: &Arc<Mutex<SessionFs>>, handle: &SharedHandle, remote: &str) -> Result<Value, String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::read_file(handle, remote).await,
        Backend::Scp => scp::read_file(handle, remote).await,
    }
}

pub async fn write_file(
    fs: &Arc<Mutex<SessionFs>>,
    handle: &SharedHandle,
    remote: &str,
    content: &str,
    mode: u32,
    base_mtime: u64,
    eol: &str,
) -> Result<Value, String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::write_file(handle, remote, content, mode, base_mtime, eol).await,
        Backend::Scp => scp::write_file(handle, remote, content, mode, base_mtime, eol).await,
    }
}

pub async fn name_conflicts(
    fs: &Arc<Mutex<SessionFs>>,
    handle: &SharedHandle,
    remote_dir: &str,
    names: &[String],
) -> Result<Vec<String>, String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::name_conflicts(handle, remote_dir, names).await,
        Backend::Scp => scp::name_conflicts(handle, remote_dir, names).await,
    }
}

pub async fn download_file(
    fs: &Arc<Mutex<SessionFs>>,
    handle: &SharedHandle,
    remote: &str,
    local: &str,
) -> Result<(), String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::download_file(handle, remote, local).await,
        Backend::Scp => scp::download_file(handle, remote, local).await,
    }
}

pub async fn put_file(
    fs: &Arc<Mutex<SessionFs>>,
    handle: &SharedHandle,
    local: &str,
    remote: &str,
) -> Result<(), String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::put_file(handle, local, remote).await,
        Backend::Scp => scp::put_file(handle, local, remote).await,
    }
}

/// Скачивание, которое бросается на ближайшем куске, как только `alive` опущен.
pub async fn download_file_while(
    fs: &Arc<Mutex<SessionFs>>,
    handle: &SharedHandle,
    remote: &str,
    local: &str,
    alive: &AtomicBool,
) -> Result<(), String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::download_file_while(handle, remote, local, Some(alive)).await,
        Backend::Scp => {
            let live = || alive.load(std::sync::atomic::Ordering::Relaxed);
            scp::download_file_ctl(handle, remote, local, &live, &mut |_: u64, _: u64| {})
                .await
                .map(|_| ())
        }
    }
}

/// Заливка, которая бросается на ближайшем куске, как только `alive` опущен.
pub async fn put_file_while(
    fs: &Arc<Mutex<SessionFs>>,
    handle: &SharedHandle,
    local: &str,
    remote: &str,
    alive: &AtomicBool,
) -> Result<(), String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::put_file_while(handle, local, remote, Some(alive)).await,
        Backend::Scp => {
            let live = || alive.load(std::sync::atomic::Ordering::Relaxed);
            scp::put_file_ctl(handle, local, remote, &live, &mut |_: u64, _: u64| {})
                .await
                .map(|_| ())
        }
    }
}

/// Всё, что нужно передаче файлов внутри одной SSH-сессии. Собирается один раз на команду
/// и дальше передаётся ссылкой - вместо восьми параметров у каждой функции.
#[derive(Clone)]
pub struct Ctx {
    pub app: AppHandle,
    pub fs: Arc<Mutex<SessionFs>>,
    pub handle: SharedHandle,
    pub session_id: String,
    /// Жива ли сессия: передача сама останавливается, когда её закрыли.
    pub alive: Arc<AtomicBool>,
    pub hub: TransferHub,
}

pub async fn upload_path(ctx: &Ctx, local: &str, remote_dir: &str) -> Result<sftp::Batch, String> {
    let (app, handle, sid, alive, hub) = (
        ctx.app.clone(),
        ctx.handle.clone(),
        ctx.session_id.as_str(),
        ctx.alive.clone(),
        ctx.hub.clone(),
    );
    match backend(&ctx.fs, &ctx.handle).await {
        Backend::Sftp => sftp::upload_path(app, handle, sid, local, remote_dir, alive, hub).await,
        Backend::Scp => scp::upload_path(app, handle, sid, local, remote_dir, alive, hub)
            .await
            .map(|()| sftp::Batch::default()),
    }
}

pub async fn download_path(ctx: &Ctx, remote: &str, local_dir: &str) -> Result<sftp::Batch, String> {
    let (app, handle, sid, alive, hub) = (
        ctx.app.clone(),
        ctx.handle.clone(),
        ctx.session_id.as_str(),
        ctx.alive.clone(),
        ctx.hub.clone(),
    );
    match backend(&ctx.fs, &ctx.handle).await {
        Backend::Sftp => sftp::download_path(app, handle, sid, remote, local_dir, alive, hub).await,
        Backend::Scp => scp::download_path(app, handle, sid, remote, local_dir, alive, hub)
            .await
            .map(|()| sftp::Batch::default()),
    }
}

/// Итог нескольких передач для журнала: успех - только если прошли все. Раньше загрузка пачки
/// писалась в журнал успешной всегда, какие бы файлы в ней ни упали.
///
/// Итоги складываются в один счёт, а не вкладываются друг в друга. Путь, который упал ещё до
/// очереди (файла нет), - одна неудача. SCP при успехе счёта не ведёт: такой путь - один файл.
pub fn batch_outcome_of(results: &[Result<sftp::Batch, String>]) -> Result<(), String> {
    let mut all = sftp::Batch::default();
    for r in results {
        match r {
            Ok(b) => {
                all.total += b.total.max(1);
                all.failed.extend(b.failed.iter().cloned());
                all.canceled += b.canceled;
            }
            Err(e) => {
                all.total += 1;
                all.failed.push(e.clone());
            }
        }
    }
    all.outcome()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(total: usize, failed: &[&str], canceled: usize) -> sftp::Batch {
        sftp::Batch {
            total,
            failed: failed.iter().map(|s| s.to_string()).collect(),
            canceled,
        }
    }

    #[test]
    fn итоги_путей_складываются_в_один_счёт() {
        assert_eq!(batch_outcome_of(&[]), Ok(()));
        assert_eq!(
            batch_outcome_of(&[Ok(batch(3, &[], 0)), Ok(sftp::Batch::default())]),
            Ok(())
        );
        assert_eq!(batch_outcome_of(&[Ok(batch(1, &[], 1))]), Err("отменено 1 из 1".into()));
        assert_eq!(
            batch_outcome_of(&[Ok(batch(3, &["нет места"], 0)), Err("нет такого файла".into())]),
            Err("не удалось 2 из 4: нет места".into())
        );
    }
}
