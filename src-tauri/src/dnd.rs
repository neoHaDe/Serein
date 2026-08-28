//! Native drag-out: мелкие файлы — OLE, крупные/отпущенная мышь — в Загрузки.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

const TMP_NAME: &str = "serein-dnd";
const MAX_AGE_SECS: u64 = 24 * 3600;
/// Держать мышь дольше этого при скачивании нереально — OLE не стартуем.
pub const OLE_MAX_BYTES: u64 = 32 * 1024 * 1024;

pub fn tmp_root() -> PathBuf {
    std::env::temp_dir().join(TMP_NAME)
}

pub fn new_tmp() -> Result<PathBuf, String> {
    let dir = tmp_root().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Сносит вчерашние каталоги переноса, чтобы %TEMP% не распухал.
pub fn cleanup_old() {
    let root = tmp_root();
    let Ok(rd) = std::fs::read_dir(&root) else {
        return;
    };
    let now = SystemTime::now();
    for ent in rd.flatten() {
        let Ok(meta) = ent.metadata() else {
            continue;
        };
        let Ok(mtime) = meta.modified() else {
            continue;
        };
        let old = now
            .duration_since(mtime)
            .map(|d| d.as_secs() > MAX_AGE_SECS)
            .unwrap_or(false);
        if old {
            let _ = std::fs::remove_dir_all(ent.path());
        }
    }
}

pub fn drag_path(p: &Path) -> Result<PathBuf, String> {
    dunce::canonicalize(p).map_err(|e| format!("{}: {e}", p.display()))
}

pub fn downloads_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::desktop_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
}

pub fn unique_in(dir: &Path, name: &str) -> PathBuf {
    let dest = dir.join(name);
    if !dest.exists() {
        return dest;
    }
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    let ext = Path::new(name).extension().map(|s| s.to_string_lossy().into_owned());
    for i in 1..1000 {
        let n = match &ext {
            Some(e) => format!("{stem} ({i}).{e}"),
            None => format!("{stem} ({i})"),
        };
        let p = dir.join(&n);
        if !p.exists() {
            return p;
        }
    }
    dest
}

fn copy_rec(src: &Path, dst: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(src).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
        for ent in std::fs::read_dir(src).map_err(|e| e.to_string())? {
            let ent = ent.map_err(|e| e.to_string())?;
            copy_rec(&ent.path(), &dst.join(ent.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(src, dst).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Переносит содержимое tmp в Загрузки (если мышь уже отпущена и OLE нельзя).
pub fn move_into_downloads(tmp: &Path) -> Result<PathBuf, String> {
    let dest_dir = downloads_dir();
    std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    let rd = std::fs::read_dir(tmp).map_err(|e| e.to_string())?;
    for ent in rd.flatten() {
        let to = unique_in(&dest_dir, &ent.file_name().to_string_lossy());
        let from = ent.path();
        if std::fs::rename(&from, &to).is_err() {
            copy_rec(&from, &to)?;
        }
    }
    let _ = std::fs::remove_dir_all(tmp);
    Ok(dest_dir)
}

pub fn mouse_left_down() -> bool {
    #[cfg(windows)]
    unsafe {
        extern "system" {
            fn GetAsyncKeyState(vKey: i32) -> i16;
        }
        GetAsyncKeyState(0x01) as u16 & 0x8000 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
pub fn start_files(
    window: &tauri::WebviewWindow,
    paths: Vec<PathBuf>,
    tmp: PathBuf,
) -> Result<(), String> {
    if paths.is_empty() {
        return Err("Нечего перетаскивать".into());
    }
    if !mouse_left_down() {
        return Err("mouseup".into());
    }
    let window = window.clone();
    let handle = window.clone();
    window
        .run_on_main_thread(move || {
            let item = drag::DragItem::Files(paths);
            let icon = drag::Image::Raw(include_bytes!("../icons/32x32.png").to_vec());
            let opts = drag::Options {
                skip_animatation_on_cancel_or_failure: true,
                mode: drag::DragMode::Copy,
            };
            let _ = drag::start_drag(
                &handle,
                item,
                icon,
                move |result, _| {
                    if matches!(result, drag::DragResult::Cancel) {
                        let _ = move_into_downloads(&tmp);
                    }
                },
                opts,
            );
        })
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn start_files(
    _window: &tauri::WebviewWindow,
    _paths: Vec<PathBuf>,
    _tmp: PathBuf,
) -> Result<(), String> {
    Err("Перетаскивание файлов наружу пока только на Windows".into())
}