//! Значок в трее: крестик прячет окна, а не закрывает приложение.
//!
//! Serein держит открытые SSH-сессии, туннели и передачи. Закрыть окно и тем самым оборвать
//! всё это одним промахом мышью - дорого, поэтому по умолчанию крестик главного окна прячет
//! приложение в трей, а выход - отдельное действие: пункт «Выйти» в меню значка или в палитре.
//!
//! Прячутся все окна сразу, а не только главное: откреплённые вкладки и панели - часть того
//! же приложения, и оставлять их висеть без главного окна странно. Возвращаются тоже все.
//!
//! На Linux сворачивание в трей по умолчанию выключено: в GNOME трея из коробки нет, и
//! спрятанное окно было бы нечем вернуть. Включается той же настройкой.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

const OPEN_ID: &str = "tray-open";
const QUIT_ID: &str = "tray-quit";

/// Значок действительно стоит. Без него прятать окна нельзя: вернуть их было бы нечем.
static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Сворачивать ли в трей по крестику: настройка или умолчание по системе.
pub fn close_to_tray() -> bool {
    if !READY.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    crate::store::settings_get()
        .get("closeToTray")
        .and_then(|v| v.as_bool())
        .unwrap_or(cfg!(windows))
}

/// Ставит значок в трей. Ошибка не фатальна: без трея приложение работает как раньше,
/// только крестик тогда закрывает его, а не прячет.
pub fn install(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, OPEN_ID, "Открыть Serein", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Выйти", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Serein")
        .menu(&menu)
        // Левый щелчок возвращает окна, меню - по правому, как у остальных программ в трее.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            OPEN_ID => show_all(app),
            QUIT_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_all(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    READY.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Показывает все окна Serein и отдаёт фокус главному.
pub fn show_all<R: Runtime>(app: &AppHandle<R>) {
    for (_, w) in app.webview_windows() {
        let _ = w.show();
        let _ = w.unminimize();
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.set_focus();
    }
}

/// Прячет все окна Serein. Сессии, туннели и передачи продолжают работать.
pub fn hide_all<R: Runtime>(app: &AppHandle<R>) {
    for (_, w) in app.webview_windows() {
        let _ = w.hide();
    }
}
