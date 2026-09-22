//! Приложение: платформа, пути профиля, выход, буфер обмена, группы окон.

use crate::{clipboard, store};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn app_platform() -> &'static str {
    #[cfg(windows)]
    {
        "windows"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        "other"
    }
}

/// Куда приложение на самом деле пишет профиль и логи сессий.
///
/// Не украшение: на Linux запуск из меню приложений и из терминала может прийти с разным
/// `HOME`/`XDG_CONFIG_HOME`, и тогда приложение молча открывает пустой профиль - список
/// серверов выглядит потерянным. По этой строке разница видна за секунду.
#[tauri::command]
pub fn app_paths() -> Value {
    let cfg = store::config_dir();
    json!({
        "config": cfg.to_string_lossy(),
        "logs": cfg.join("logs").to_string_lossy(),
    })
}

/// Выход из приложения. Крестик прячет окна в трей, поэтому выходу нужно своё действие.
#[tauri::command]
pub fn app_quit(app: AppHandle) {
    app.exit(0);
}

/// Как приложение установлено - от этого зависит, можно ли обновиться на месте.
///
/// `installer` - Windows: апдейтер скачивает установщик и перезапускает приложение.
/// `appimage` - Linux, запуск из AppImage: файл заменяется целиком, это единственная
/// форма на Linux, которую умеет обновлять сам Tauri (переменную `APPIMAGE` выставляет
/// среда выполнения AppImage).
/// `package` - Linux из `.deb`: бинарь лежит в `/usr/bin` и принадлежит менеджеру пакетов,
/// писать туда приложение не может и не должно. Обновление - через пакет.
#[tauri::command]
pub fn app_install_kind() -> &'static str {
    #[cfg(windows)]
    {
        "installer"
    }
    #[cfg(not(windows))]
    {
        if std::env::var_os("APPIMAGE").is_some() {
            "appimage"
        } else {
            "package"
        }
    }
}

#[tauri::command]
pub fn clipboard_write(text: String) -> Result<(), String> {
    clipboard::write_text(&text)
}

#[tauri::command]
pub fn clipboard_read() -> Result<String, String> {
    clipboard::read_text()
}

#[tauri::command]
pub fn export_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content.as_bytes()).map_err(|e| e.to_string())
}

/// Сдвинуть окна группы на (dx, dy) в физических пикселях.
/// Делаем из Rust: на Linux JS `setPosition` из чужого webview часто не доезжает,
/// а emit `serein-dock-move` сам по себе окна не двигает - только помечает «это наше».
#[tauri::command]
pub fn windows_nudge_group(app: AppHandle, members: Vec<String>, dx: i32, dy: i32) {
    if dx == 0 && dy == 0 {
        return;
    }
    use tauri::Manager;
    for label in members {
        let Some(w) = app.webview_windows().get(&label).cloned() else {
            continue;
        };
        let Ok(pos) = w.outer_position() else {
            continue;
        };
        let _ = w.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
            x: pos.x.saturating_add(dx),
            y: pos.y.saturating_add(dy),
        }));
    }
}

/// Поднять все окна приложения над чужими, фокус оставить на `focused`.
#[tauri::command]
pub fn windows_raise_group(app: AppHandle, focused: String) {
    windows_raise_group_impl(&app, &focused);
}

/// Развернуть все свёрнутые окна приложения (для режима «одна кнопка на панели задач»).
#[tauri::command]
pub fn windows_restore_minimized(app: AppHandle) -> u32 {
    windows_restore_minimized_impl(&app)
}

/// Сколько окон приложения сейчас свёрнуто.
#[tauri::command]
pub fn windows_count_minimized(app: AppHandle) -> u32 {
    windows_count_minimized_impl(&app)
}

// На Linux те же три операции делаются средствами самого Tauri. Раньше они там были
// заглушками, из-за чего «одна кнопка на панели задач» и подъём группы не работали
// вовсе: свёрнутые окна не разворачивались, а клик по одному окну не поднимал остальные.

#[cfg(not(windows))]
fn windows_count_minimized_impl(app: &AppHandle) -> u32 {
    app.webview_windows()
        .values()
        .filter(|w| w.is_minimized().unwrap_or(false))
        .count() as u32
}

#[cfg(not(windows))]
fn windows_restore_minimized_impl(app: &AppHandle) -> u32 {
    let mut n = 0;
    for w in app.webview_windows().values() {
        if w.is_minimized().unwrap_or(false) {
            let _ = w.unminimize();
            n += 1;
        }
    }
    n
}

#[cfg(not(windows))]
fn windows_raise_group_impl(app: &AppHandle, focused: &str) {
    // В Win32 есть отдельное «поднять, не забирая фокус» (SWP_NOACTIVATE); в Tauri его нет,
    // а `set_focus` перетащил бы фокус на каждое окно по очереди и заставил их мигать.
    // Переносимый эквивалент - короткое «поверх всех» и обратно: менеджер окон поднимает
    // окно, фокус остаётся там, где был.
    let windows = app.webview_windows();
    for (label, w) in &windows {
        if label == focused || w.is_minimized().unwrap_or(false) {
            continue;
        }
        let _ = w.set_always_on_top(true);
        let _ = w.set_always_on_top(false);
    }
    // Фокусируемое поднимаем последним, чтобы оно осталось верхним и активным.
    if let Some(w) = windows.get(focused) {
        let _ = w.set_focus();
    }
}

#[cfg(windows)]
fn windows_count_minimized_impl(app: &AppHandle) -> u32 {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsIconic;

    let mut n = 0u32;
    for (_, w) in app.webview_windows() {
        let Ok(h) = w.hwnd() else { continue };
        let hwnd = HWND(h.0 as isize as *mut c_void);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                n += 1;
            }
        }
    }
    n
}

#[cfg(windows)]
fn windows_restore_minimized_impl(app: &AppHandle) -> u32 {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, ShowWindow, SW_RESTORE};

    let mut n = 0u32;
    for (_, w) in app.webview_windows() {
        let Ok(h) = w.hwnd() else { continue };
        let hwnd = HWND(h.0 as isize as *mut c_void);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                n += 1;
            }
        }
    }
    n
}

#[cfg(windows)]
fn windows_raise_group_impl(app: &AppHandle, focused: &str) {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SetForegroundWindow, SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    let mut others: Vec<HWND> = Vec::new();
    let mut focus_hwnd: Option<HWND> = None;

    for (label, w) in app.webview_windows() {
        let Ok(h) = w.hwnd() else { continue };
        let hwnd = HWND(h.0 as isize as *mut c_void);
        // Свернутые окна не трогаем - иначе minimize сразу отменяется raise_group.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                continue;
            }
        }
        if label == focused {
            focus_hwnd = Some(hwnd);
        } else {
            others.push(hwnd);
        }
    }

    unsafe {
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        for hwnd in others {
            let _ = SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, flags);
        }
        if let Some(hwnd) = focus_hwnd {
            let _ = SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, flags);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}
