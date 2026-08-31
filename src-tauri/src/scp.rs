//! SCP поверх SSH exec (`scp -f` / `scp -t`) и ls/exec для каталогов без SFTP.

use crate::sftp::{check_remote_path, dup_key, emit_transfer, join_remote, TransferHub, CANCELLED};
use crate::ssh::{ClientHandler, SharedHandle};
use base64::{engine::general_purpose::STANDARD, Engine};
use russh::client;
use russh::{Channel, ChannelMsg};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::AppHandle;
const SCP_CHUNK: usize = 64 * 1024;
const MAX_PREVIEW: u64 = 8 * 1024 * 1024;
const MAX_EDIT_SIZE: u64 = 5 * 1024 * 1024;

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

struct ScpIo {
    channel: Channel<client::Msg>,
    buf: Vec<u8>,
}

impl ScpIo {
    async fn open(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, cmd: &str) -> Result<Self, String> {
        let channel = {
            let h = handle.lock().await;
            h.channel_open_session().await.map_err(|e| e.to_string())?
        };
        channel.exec(true, cmd).await.map_err(|e| e.to_string())?;
        Ok(Self { channel, buf: Vec::new() })
    }

    async fn read_byte(&mut self) -> Result<u8, String> {
        loop {
            if !self.buf.is_empty() {
                return Ok(self.buf.remove(0));
            }
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => self.buf.extend_from_slice(&data),
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    return Err(format!("SCP: канал закрыт (код {exit_status})"));
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                    return Err("SCP: неожиданный конец канала".into());
                }
                _ => {}
            }
        }
    }

    async fn read_ack(&mut self) -> Result<(), String> {
        match self.read_byte().await? {
            0 => Ok(()),
            1 => Ok(()),
            2 => Err("SCP: удалённая ошибка".into()),
            b => Err(format!("SCP: неверный ack ({b})")),
        }
    }

    async fn send_ack(&mut self) -> Result<(), String> {
        self.channel.data(&[0u8][..]).await.map_err(|e| e.to_string())
    }

    async fn read_line(&mut self) -> Result<String, String> {
        loop {
            if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
                let line = String::from_utf8_lossy(&self.buf[..i]).to_string();
                self.buf.drain(..=i);
                return Ok(line);
            }
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => self.buf.extend_from_slice(&data),
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    return Err(format!("SCP: канал закрыт (код {exit_status})"));
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                    return Err("SCP: неожиданный конец канала".into());
                }
                _ => {}
            }
        }
    }

    async fn read_exact(&mut self, n: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            if !self.buf.is_empty() {
                let take = (n - out.len()).min(self.buf.len());
                out.extend_from_slice(&self.buf[..take]);
                self.buf.drain(..take);
                continue;
            }
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => self.buf.extend_from_slice(&data),
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    return Err(format!("SCP: канал закрыт (код {exit_status})"));
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                    return Err("SCP: неожиданный конец канала".into());
                }
                _ => {}
            }
        }
        Ok(out)
    }

    async fn write_all(&mut self, data: &[u8]) -> Result<(), String> {
        for chunk in data.chunks(SCP_CHUNK) {
            self.channel.data(chunk).await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn perm_to_mode(perm: &str) -> u32 {
    if perm.len() < 10 {
        return 0o644;
    }
    fn tri(p: &str, i: usize) -> u32 {
        let mut m = 0u32;
        let bytes = p.as_bytes();
        if bytes.get(i).copied() == Some(b'r') {
            m |= 4;
        }
        if bytes.get(i + 1).copied() == Some(b'w') {
            m |= 2;
        }
        let x = bytes.get(i + 2).copied();
        if x == Some(b'x') || x == Some(b's') || x == Some(b't') {
            m |= 1;
        }
        m
    }
    tri(perm, 1) * 64 + tri(perm, 4) * 8 + tri(perm, 7)
}

/// Разбор строки `ls -lan`.
pub fn parse_ls_line(line: &str) -> Option<(char, u64, u32, u64, String, Option<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("total ") {
        return None;
    }
    let kind = line.chars().next()?;
    if kind == 't' {
        return None;
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 9 {
        return None;
    }
    let size: u64 = parts[4].parse().ok()?;
    let mode = perm_to_mode(parts[0]);
    let mtime = 0u64;
    let name_part = parts[8..].join(" ");
    if name_part == "." || name_part == ".." {
        return None;
    }
    let (name, target) = if let Some((n, t)) = name_part.split_once(" -> ") {
        (n.trim().to_string(), Some(t.trim().to_string()))
    } else {
        (name_part, None)
    };
    Some((kind, size, mode, mtime, name, target))
}

async fn canonical_dir(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str) -> Result<String, String> {
    let p = if path.is_empty() || path == "." {
        "/".to_string()
    } else {
        path.to_string()
    };
    check_remote_path(&p)?;
    if p == "/" {
        return Ok("/".into());
    }
    let cmd = format!("cd -- {} && pwd -P", shell_quote(&p));
    let (code, out, err) = crate::ssh::exec(handle, &cmd, None).await?;
    if code != 0 {
        return Err(if err.trim().is_empty() {
            format!("Каталог «{p}» недоступен (код {code})")
        } else {
            err.trim().to_string()
        });
    }
    Ok(out.trim().to_string())
}

async fn run_sh(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, script: &str) -> Result<(), String> {
    let (code, _out, err) = crate::ssh::exec(handle, script, None).await?;
    if code != 0 {
        return Err(if err.trim().is_empty() {
            format!("Команда завершилась с кодом {code}")
        } else {
            err.trim().to_string()
        });
    }
    Ok(())
}

pub async fn list(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str) -> Result<Value, String> {
    let abs = canonical_dir(handle, path).await?;
    let cmd = format!("LC_ALL=C ls -lan -- {}", shell_quote(&abs));
    let (code, out, err) = crate::ssh::exec(handle, &cmd, None).await?;
    if code != 0 {
        return Err(if err.trim().is_empty() {
            format!("ls завершился с кодом {code}")
        } else {
            err.trim().to_string()
        });
    }
    let mut entries: Vec<Value> = Vec::new();
    for line in out.lines() {
        let Some((kind, size, mode, mtime, name, target)) = parse_ls_line(line) else {
            continue;
        };
        let entry_type = match kind {
            'd' => "dir",
            'l' => "link",
            '-' => "file",
            _ => continue,
        };
        let (link_type, target_val) = if entry_type == "link" {
            (
                target.as_ref().map(|_| "file"),
                target,
            )
        } else {
            (None, None)
        };
        entries.push(json!({
            "name": name,
            "type": entry_type,
            "size": size,
            "mtime": mtime,
            "mode": mode,
            "target": target_val,
            "linkType": link_type,
        }));
    }
    entries.sort_by(|a, b| {
        let ad = a["type"] == json!("dir");
        let bd = b["type"] == json!("dir");
        bd.cmp(&ad).then_with(|| {
            a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or(""))
        })
    });
    Ok(json!({ "path": abs, "entries": entries, "backend": "scp" }))
}

async fn recv_file_bytes(io: &mut ScpIo) -> Result<Vec<u8>, String> {
    io.read_ack().await?;
    io.send_ack().await?;
    let line = io.read_line().await?;
    if line == "\x04" || line.is_empty() {
        return Err("SCP: файл не передан".into());
    }
    if !line.starts_with('C') {
        return Err(format!("SCP: ожидали файл, получили «{line}»"));
    }
    let rest = line[1..].trim();
    let mut parts = rest.splitn(3, ' ');
    let _mode = parts.next().unwrap_or("");
    let size: usize = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("SCP: неверный размер в «{line}»"))?;
    io.send_ack().await?;
    let data = io.read_exact(size).await?;
    let pad = io.read_byte().await?;
    if pad != 0 {
        return Err("SCP: неверный терминатор данных".into());
    }
    io.send_ack().await?;
    Ok(data)
}

pub async fn download_file(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &str,
    local: &str,
) -> Result<(), String> {
    check_remote_path(remote)?;
    let cmd = format!("scp -f -- {}", shell_quote(remote));
    let mut io = ScpIo::open(handle, &cmd).await?;
    let data = recv_file_bytes(&mut io).await?;
    if let Some(parent) = Path::new(local).parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    tokio::fs::write(local, &data).await.map_err(|e| e.to_string())
}

async fn send_file_bytes(io: &mut ScpIo, name: &str, data: &[u8], mode: u32) -> Result<(), String> {
    io.read_ack().await?;
    let header = format!("C{:04o} {} {}\n", mode & 0o777, data.len(), name);
    io.write_all(header.as_bytes()).await?;
    io.read_ack().await?;
    io.write_all(data).await?;
    io.channel.data(&[0u8][..]).await.map_err(|e| e.to_string())?;
    io.read_ack().await?;
    Ok(())
}

pub async fn put_file(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    local: &str,
    remote: &str,
) -> Result<(), String> {
    check_remote_path(remote)?;
    let data = tokio::fs::read(local).await.map_err(|e| e.to_string())?;
    let mode = tokio::fs::metadata(local)
        .await
        .ok()
        .map(|m| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() & 0o777
            }
            #[cfg(not(unix))]
            {
                let _ = m;
                0o644u32
            }
        })
        .unwrap_or(0o644);
    let parent = Path::new(remote)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".into());
    let fname = Path::new(remote)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let cmd = format!("scp -t -- {}", shell_quote(&parent));
    let mut io = ScpIo::open(handle, &cmd).await?;
    send_file_bytes(&mut io, &fname, &data, mode).await
}

pub async fn mkdir(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str) -> Result<(), String> {
    check_remote_path(path)?;
    run_sh(handle, &format!("mkdir -p -- {}", shell_quote(path))).await
}

pub async fn remove(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str, is_dir: bool) -> Result<(), String> {
    check_remote_path(path)?;
    if is_dir {
        run_sh(handle, &format!("rm -rf -- {}", shell_quote(path))).await
    } else {
        run_sh(handle, &format!("rm -f -- {}", shell_quote(path))).await
    }
}

pub async fn rename(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    from: &str,
    to: &str,
) -> Result<(), String> {
    check_remote_path(from)?;
    check_remote_path(to)?;
    run_sh(
        handle,
        &format!("mv -- {} {}", shell_quote(from), shell_quote(to)),
    )
    .await
}

pub async fn chmod(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str, mode: u32) -> Result<(), String> {
    check_remote_path(path)?;
    run_sh(
        handle,
        &format!("chmod {:o} -- {}", mode & 0o777, shell_quote(path)),
    )
    .await
}

pub async fn preview(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, remote: &str) -> Result<Value, String> {
    check_remote_path(remote)?;
    let cmd = format!("wc -c < -- {}", shell_quote(remote));
    let (code, out, err) = crate::ssh::exec(handle, &cmd, None).await?;
    if code != 0 {
        return Err(err.trim().to_string());
    }
    let size: u64 = out.trim().parse().unwrap_or(0);
    if size > MAX_PREVIEW {
        return Ok(json!({ "kind": "tooLarge", "size": size }));
    }
    let cmd = format!("scp -f -- {}", shell_quote(remote));
    let mut io = ScpIo::open(handle, &cmd).await?;
    let buf = recv_file_bytes(&mut io).await?;
    Ok(json!({
        "kind": "bytes",
        "size": size,
        "base64": STANDARD.encode(&buf)
    }))
}

pub async fn read_file(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, remote: &str) -> Result<Value, String> {
    check_remote_path(remote)?;
    let cmd = format!("wc -c < -- {}", shell_quote(remote));
    let (code, out, _err) = crate::ssh::exec(handle, &cmd, None).await?;
    let size: u64 = if code == 0 { out.trim().parse().unwrap_or(0) } else { 0 };
    let mode = 0o644u32;
    let mtime = 0u64;
    if size > MAX_EDIT_SIZE {
        return Ok(json!({ "content": "", "eol": "lf", "mode": mode, "mtime": mtime, "tooLarge": true }));
    }
    let cmd = format!("scp -f -- {}", shell_quote(remote));
    let mut io = ScpIo::open(handle, &cmd).await?;
    let buf = recv_file_bytes(&mut io).await?;
    if buf.iter().take(8192).any(|b| *b == 0) {
        return Ok(json!({ "content": "", "eol": "lf", "mode": mode, "mtime": mtime, "binary": true }));
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let eol = if text.contains("\r\n") { "crlf" } else { "lf" };
    let content = if eol == "crlf" { text.replace("\r\n", "\n") } else { text };
    Ok(json!({ "content": content, "eol": eol, "mode": mode, "mtime": mtime }))
}

pub async fn write_file(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &str,
    content: &str,
    mode: u32,
    base_mtime: u64,
    eol: &str,
) -> Result<Value, String> {
    check_remote_path(remote)?;
    let _ = base_mtime;
    let data = if eol == "crlf" {
        content.replace('\n', "\r\n")
    } else {
        content.to_string()
    };
    let dir = Path::new(remote)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "/".into());
    let base = Path::new(remote)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let tmp_name = format!(".{base}.terminal-tmp");
    let tmp = join_remote(&dir, &tmp_name);
    let cmd = format!("scp -t -- {}", shell_quote(&dir));
    let mut io = ScpIo::open(handle, &cmd).await?;
    send_file_bytes(&mut io, &tmp_name, data.as_bytes(), mode).await?;
    rename(handle, &tmp, remote).await?;
    Ok(json!({ "ok": true, "mtime": base_mtime }))
}

pub async fn name_conflicts(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote_dir: &str,
    names: &[String],
) -> Result<Vec<String>, String> {
    let listed = list(handle, remote_dir).await?;
    let existing: std::collections::HashSet<String> = listed["entries"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|e| e["name"].as_str().map(str::to_string))
        .collect();
    Ok(names
        .iter()
        .filter_map(|raw| {
            let norm = raw.replace('\\', "/");
            let name = norm.rsplit('/').next()?.to_string();
            if name.is_empty() || name == "." || name == ".." {
                return None;
            }
            existing.contains(&name).then_some(name)
        })
        .collect())
}

async fn collect_local(
    local: &str,
    remote: &str,
    rel: &str,
    out: &mut Vec<(String, String, String, u64)>,
) -> Result<(), String> {
    let meta = tokio::fs::metadata(local).await.map_err(|e| e.to_string())?;
    if meta.is_dir() {
        let mut rd = tokio::fs::read_dir(local).await.map_err(|e| e.to_string())?;
        while let Some(entry) = rd.next_entry().await.map_err(|e| e.to_string())? {
            let name = entry.file_name().to_string_lossy().to_string();
            let lp = format!("{local}/{name}");
            let rp = join_remote(remote, &name);
            let r = format!("{rel}/{name}");
            Box::pin(collect_local(&lp, &rp, &r, out)).await?;
        }
    } else {
        out.push((
            local.to_string(),
            remote.to_string(),
            rel.to_string(),
            meta.len(),
        ));
    }
    Ok(())
}

pub async fn upload_path(
    app: AppHandle,
    handle: SharedHandle,
    session_id: &str,
    local: &str,
    remote_dir: &str,
    alive: Arc<AtomicBool>,
    hub: TransferHub,
) -> Result<(), String> {
    if !alive.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    check_remote_path(remote_dir)?;
    let local = local.replace('\\', "/");
    let root_name = Path::new(&local)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let mut files: Vec<(String, String, String, u64)> = Vec::new();
    collect_local(&local, &join_remote(remote_dir, &root_name), &root_name, &mut files).await?;
    for (lp, rp, rel, size) in files {
        if !alive.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let key = dup_key(session_id, "upload", &lp, &rp);
        let id = uuid::Uuid::new_v4().to_string();
        let Some(ctrl) = hub.start(&id, session_id, key) else {
            continue;
        };
        emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "active", None);
        let result = put_file(handle.as_ref(), &lp, &rp).await;
        hub.finish(&id);
        if !ctrl.is_live() {
            emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "canceled", None);
            return Err(CANCELLED.into());
        }
        match result {
            Ok(()) => emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, size, "done", None),
            Err(e) if e == CANCELLED => {
                emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "canceled", None);
                return Err(e);
            }
            Err(e) => {
                emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "error", Some(&e));
                return Err(e);
            }
        }
    }
    Ok(())
}

async fn collect_remote_list(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &str,
    local: &str,
    rel: &str,
    out: &mut Vec<(String, String, String, u64)>,
) -> Result<(), String> {
    let listed = list(handle, remote).await?;
    let abs = listed["path"].as_str().unwrap_or(remote);
    for entry in listed["entries"].as_array().unwrap_or(&vec![]).clone() {
        let name = entry["name"].as_str().unwrap_or("").to_string();
        let rp = join_remote(abs, &name);
        let lp = format!("{local}/{name}");
        let r = format!("{rel}/{name}");
        if entry["type"] == json!("dir") {
            Box::pin(collect_remote_list(handle, &rp, &lp, &r, out)).await?;
        } else if entry["type"] == json!("file") {
            let size = entry["size"].as_u64().unwrap_or(0);
            out.push((lp, rp, r, size));
        }
    }
    Ok(())
}

async fn remote_is_dir(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, remote: &str) -> Result<bool, String> {
    let cmd = format!(
        "if [ -d {} ]; then echo dir; elif [ -f {} ]; then echo file; else echo no; fi",
        shell_quote(remote),
        shell_quote(remote)
    );
    let (code, out, err) = crate::ssh::exec(handle, &cmd, None).await?;
    if code != 0 {
        return Err(err.trim().to_string());
    }
    match out.trim() {
        "dir" => Ok(true),
        "file" => Ok(false),
        _ => Err(format!("Путь «{remote}» не найден")),
    }
}

pub async fn download_path(
    app: AppHandle,
    handle: SharedHandle,
    session_id: &str,
    remote: &str,
    local_dir: &str,
    alive: Arc<AtomicBool>,
    hub: TransferHub,
) -> Result<(), String> {
    if !alive.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    check_remote_path(remote)?;
    let local_dir = local_dir.replace('\\', "/");
    let mut jobs: Vec<(String, String, String, u64)> = Vec::new();
    if remote_is_dir(handle.as_ref(), remote).await? {
        let base = Path::new(remote)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "download".into());
        let local_root = format!("{local_dir}/{base}");
        collect_remote_list(handle.as_ref(), remote, &local_root, &base, &mut jobs).await?;
    } else {
        let name = Path::new(remote)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        jobs.push((format!("{local_dir}/{name}"), remote.to_string(), name, 0));
    }
    for (lp, rp, rel, size) in jobs {
        if !alive.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        if let Some(parent) = Path::new(&lp).parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let key = dup_key(session_id, "download", &lp, &rp);
        let id = uuid::Uuid::new_v4().to_string();
        let Some(ctrl) = hub.start(&id, session_id, key) else {
            continue;
        };
        emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "active", None);
        let result = download_file(handle.as_ref(), &rp, &lp).await;
        hub.finish(&id);
        if !ctrl.is_live() {
            emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "canceled", None);
            return Err(CANCELLED.into());
        }
        match result {
            Ok(()) => emit_transfer(
                &app,
                &id,
                session_id,
                "download",
                &lp,
                &rp,
                &rel,
                size,
                size.max(1),
                "done",
                None,
            ),
            Err(e) if e == CANCELLED => {
                emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "canceled", None);
                return Err(e);
            }
            Err(e) => {
                emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "error", Some(&e));
                return Err(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ls_file_and_dir() {
        let f = parse_ls_line("-rw-r--r--   1 0 0     1234 Aug 31 13:00 notes.txt").unwrap();
        assert_eq!(f.0, '-');
        assert_eq!(f.1, 1234);
        assert_eq!(f.4, "notes.txt");
        let d = parse_ls_line("drwxr-xr-x   2 0 0     4096 Aug 31 13:00 srv").unwrap();
        assert_eq!(d.0, 'd');
        assert_eq!(d.4, "srv");
    }

    #[test]
    fn parse_ls_symlink() {
        let l = parse_ls_line("lrwxrwxrwx   1 0 0       11 Aug 31 13:00 link -> /etc").unwrap();
        assert_eq!(l.0, 'l');
        assert_eq!(l.4, "link");
        assert_eq!(l.5.as_deref(), Some("/etc"));
    }
}
