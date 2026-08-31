//! Файловый менеджер: SFTP, при недоступности подсистемы — SCP + ls/exec.

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

pub async fn download_file(fs: &Arc<Mutex<SessionFs>>, handle: &SharedHandle, remote: &str, local: &str) -> Result<(), String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::download_file(handle, remote, local).await,
        Backend::Scp => scp::download_file(handle, remote, local).await,
    }
}

pub async fn put_file(fs: &Arc<Mutex<SessionFs>>, handle: &SharedHandle, local: &str, remote: &str) -> Result<(), String> {
    match backend(fs, handle).await {
        Backend::Sftp => sftp::put_file(handle, local, remote).await,
        Backend::Scp => scp::put_file(handle, local, remote).await,
    }
}

pub async fn upload_path(
    app: AppHandle,
    fs: Arc<Mutex<SessionFs>>,
    handle: SharedHandle,
    session_id: &str,
    local: &str,
    remote_dir: &str,
    alive: Arc<AtomicBool>,
    hub: TransferHub,
) -> Result<(), String> {
    match backend(&fs, &handle).await {
        Backend::Sftp => sftp::upload_path(app, handle, session_id, local, remote_dir, alive, hub).await,
        Backend::Scp => scp::upload_path(app, handle, session_id, local, remote_dir, alive, hub).await,
    }
}

pub async fn download_path(
    app: AppHandle,
    fs: Arc<Mutex<SessionFs>>,
    handle: SharedHandle,
    session_id: &str,
    remote: &str,
    local_dir: &str,
    alive: Arc<AtomicBool>,
    hub: TransferHub,
) -> Result<(), String> {
    match backend(&fs, &handle).await {
        Backend::Sftp => sftp::download_path(app, handle, session_id, remote, local_dir, alive, hub).await,
        Backend::Scp => scp::download_path(app, handle, session_id, remote, local_dir, alive, hub).await,
    }
}
