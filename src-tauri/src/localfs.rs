//! Локальная файловая система для двухпанельного SFTP-менеджера.

use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub fn home() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".into())
}

pub fn parent(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

pub fn list(path: &str) -> Result<Value, String> {
    let p = if path.is_empty() { home() } else { path.to_string() };
    let mut entries: Vec<Value> = Vec::new();
    let rd = fs::read_dir(&p).map_err(|e| e.to_string())?;
    for ent in rd.flatten() {
        let meta = match ent.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let kind = if meta.is_dir() {
            "dir"
        } else if meta.is_file() {
            "file"
        } else {
            "other"
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        entries.push(json!({
            "name": ent.file_name().to_string_lossy(),
            "type": kind,
            "size": meta.len(),
            "mtime": mtime,
        }));
    }
    // Папки сверху, затем по имени.
    entries.sort_by(|a, b| {
        let ad = a["type"] == json!("dir");
        let bd = b["type"] == json!("dir");
        bd.cmp(&ad).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
        })
    });
    Ok(json!({ "path": p, "entries": entries }))
}

/// Копирует файлы/папки в dest_dir (drop из Проводника в локальную панель).
pub fn copy_into(paths: &[String], dest_dir: &str) -> Result<u32, String> {
    let dest = Path::new(dest_dir);
    if !dest.is_dir() {
        return Err("Папка назначения не найдена".into());
    }
    let mut n = 0u32;
    for p in paths {
        let src = Path::new(p);
        let name = src
            .file_name()
            .ok_or_else(|| format!("Некорректный путь: {p}"))?;
        copy_item(src, &dest.join(name))?;
        n += 1;
    }
    Ok(n)
}

fn copy_item(src: &Path, dst: &Path) -> Result<(), String> {
    let meta = fs::metadata(src).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        fs::create_dir_all(dst).map_err(|e| e.to_string())?;
        for ent in fs::read_dir(src).map_err(|e| e.to_string())? {
            let ent = ent.map_err(|e| e.to_string())?;
            copy_item(&ent.path(), &dst.join(ent.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(src, dst).map_err(|e| e.to_string())?;
        Ok(())
    }
}
