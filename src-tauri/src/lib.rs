//! Serein - backend. Состояние приложения (сессии и всё, что к ним привязано) и запуск.
//! Команды Tauri - в `commands`, по предметным областям.

mod actionlog;
mod backup;
mod chain;
mod clipboard;
mod commands;
mod crypto;
pub mod db;
pub mod deskout;
mod dnd;
mod docker;
mod docker_compose;
mod dpapi;
mod error;
pub mod filediff;
mod foldersync;
mod importers;
mod keygen;
mod knownhosts;
pub mod ldap;
mod localfs;
pub mod localname;
pub mod metrics;
pub mod mongo;
pub mod monitor;
mod multihost;
pub mod mysql;
mod os_secrets;
mod ownership;
mod paths;
pub mod platform;
pub mod policy;
pub mod profile_lock;
mod proxycmd;
mod pty;
pub mod rdp;
mod rdp_capture;
pub mod rdpsetup;
pub mod remote_fs;
mod remoteedit;
mod schema;
pub mod scp;
mod serial;
pub mod sftp;
pub mod ssh;
mod ssh_agent;
mod ssh_algos;
pub mod store;
mod sync;
pub mod sysinfo;
mod tasks;
mod telnet;
mod term_out;
mod termsize;
pub mod tools;
mod tray;
mod tunnels;
mod vault;
mod vaultkey;
pub mod vnc;
pub mod vncsetup;
pub mod workspace;

use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

pub(crate) enum Session {
    Local(pty::LocalSession),
    Ssh(Arc<ssh::SshSession>),
    Serial(serial::SerialSession),
    /// Telnet или «сырой» TCP - общий транспорт, разный разбор потока.
    Tcp(telnet::TcpSession),
}

pub(crate) struct AppState {
    sessions: Mutex<HashMap<String, Session>>,
    ki: ssh::KiBridge,
    host_keys: ssh::HostKeyBridge,
    tunnels: tunnels::TunnelManager,
    edit: remoteedit::EditManager,
    transfers: sftp::TransferHub,
    ops: ssh::OpHub,
    /// Какому окну принадлежит сессия. Закрыть её может только владелец - см. `ownership`.
    owners: ownership::Owners,
}

impl AppState {
    fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            ki: Arc::new(Mutex::new(HashMap::new())),
            host_keys: Arc::new(Mutex::new(HashMap::new())),
            tunnels: tunnels::TunnelManager::default(),
            edit: remoteedit::EditManager::default(),
            transfers: sftp::TransferHub::default(),
            ops: ssh::OpHub::default(),
            owners: ownership::Owners::default(),
        }
    }
    fn ssh(&self, id: &str) -> Option<Arc<ssh::SshSession>> {
        match crate::sync::lock(&self.sessions).get(id) {
            Some(Session::Ssh(s)) => Some(s.clone()),
            _ => None,
        }
    }

    /// Идемпотентно: туннели, edit-watchers, KI, russh disconnect. Можно звать с фронта и из shell-таска.
    pub(crate) fn teardown(&self, app: &AppHandle, id: &str, user: bool) {
        if let Some(server) = actionlog::unbind(id) {
            let reason = if user { "closed" } else { "drop" };
            actionlog::record(
                Some(&server),
                Some(id),
                "ssh.disconnect",
                json!({ "reason": reason }),
                Ok(()),
            );
        }
        // Рабочий стол ходит своим SSH-соединением и сам со смертью сессии не умрёт.
        // Открыт он был из неё и по её учётке - без неё жить не должен.
        commands::desktop::close_desktop_of(id);
        self.tunnels.close_session(id, app);
        self.edit.stop_session(id);
        self.transfers.cancel_session(id);
        self.ops.cancel_prefix(&format!("{id}:"));
        term_out::replay_forget(id);
        platform::forget(id);
        commands::observability::sysinfo_forget(id);
        metrics::forget(id);
        db::close_session(id);
        self.owners.release(id);
        if let Some(tx) = crate::sync::lock(&self.ki).remove(id) {
            drop(tx);
        }
        if let Some(s) = crate::sync::lock(&self.sessions).remove(id) {
            match s {
                Session::Local(l) => l.close(),
                Session::Serial(p) => p.close(),
                Session::Tcp(t) => t.close(),
                Session::Ssh(s) => s.shutdown(user),
            }
        }
    }
}

/// WebView2 по умолчанию вешает Ctrl+Shift+C на Inspect - это ломает копирование в терминале.
#[cfg(windows)]
fn disable_browser_accelerators(w: &tauri::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
    use windows_core::Interface;
    let _ = w.with_webview(|wv| {
        let controller = wv.controller();
        unsafe {
            let Ok(core) = controller.CoreWebView2() else {
                return;
            };
            let Ok(settings) = core.Settings() else {
                return;
            };
            let Ok(s3) = settings.cast::<ICoreWebView2Settings3>() else {
                return;
            };
            let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false);
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Один процесс на профиль - раньше всего, что профиль читает или пишет: миграция
            // схемы ниже уже пишет. Почему это нужно - в `profile_lock`.
            if let Err(busy) = profile_lock::hold(&store::config_dir()) {
                use tauri_plugin_dialog::DialogExt;
                let _ = app.dialog().message(&busy).title("Serein уже запущен").blocking_show();
                // Осознанный отказ, а не сбой: выходим тихо, без паники в журнале.
                std::process::exit(0);
            }
            // Схему профиля приводим к текущей до того, как что-либо его прочитает.
            // Ошибка здесь означает профиль от более новой версии: продолжать нельзя -
            // первая же запись выбросит поля, которых мы не знаем.
            if let Err(e) = schema::migrate(&store::config_dir()) {
                use tauri_plugin_dialog::DialogExt;
                let _ = app
                    .dialog()
                    .message(&e)
                    .title("Serein - профиль несовместим")
                    .blocking_show();
                return Err(e.into());
            }
            app.manage(AppState::new());
            if let Err(e) = tray::install(app) {
                // Без значка крестик просто закрывает приложение - как было до трея.
                eprintln!("значок в трее не поставлен: {e}");
            }
            actionlog::init(&store::settings_get());
            actionlog::record(
                None,
                None,
                "app.start",
                json!({ "version": app.package_info().version.to_string(), "policy": policy::status() }),
                match &policy::current().error {
                    Some(e) => Err(format!("политика администратора не применена: {e}")),
                    None => Ok(()),
                },
            );
            #[cfg(windows)]
            if let Some(w) = app.get_webview_window("main") {
                disable_browser_accelerators(&w);
            }
            Ok(())
        })
        // Крестик главного окна (и Alt+F4) прячет приложение в трей, а не закрывает его: так
        // один промах мышью не обрывает сессии, туннели и передачи. Подробности - в `tray`.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && tray::close_to_tray() {
                    api.prevent_close();
                    tray::hide_all(window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::export_text_file,
            commands::app::app_platform,
            commands::app::app_paths,
            commands::app::app_install_kind,
            commands::app::app_quit,
            commands::app::windows_nudge_group,
            commands::app::windows_raise_group,
            commands::app::windows_restore_minimized,
            commands::app::windows_count_minimized,
            commands::app::clipboard_write,
            commands::app::clipboard_read,
            commands::profile::settings_get,
            commands::profile::settings_set,
            commands::profile::servers_list,
            commands::profile::servers_save,
            commands::profile::servers_delete,
            commands::profile::servers_reorder,
            commands::profile::snippets_list,
            commands::profile::snippets_save,
            commands::profile::snippets_delete,
            commands::profile::workspaces_list,
            commands::profile::workspaces_save,
            commands::profile::workspaces_delete,
            commands::profile::layout_get,
            commands::profile::layout_set,
            commands::profile::aux_layout_get,
            commands::profile::aux_layout_set,
            commands::profile::vault_status,
            commands::profile::vault_unlock,
            commands::profile::vault_enable,
            commands::profile::vault_disable,
            commands::profile::backup_export,
            commands::profile::backup_preview,
            commands::profile::backup_import,
            commands::profile::keygen_generate,
            commands::profile::keygen_save,
            commands::profile::keygen_install,
            commands::profile::servers_import_ssh_config,
            commands::profile::servers_import_putty,
            commands::profile::servers_import_mobaxterm,
            commands::profile::servers_import_xshell,
            commands::profile::servers_import_securecrt,
            commands::session::session_open_local,
            commands::session::session_open_ssh,
            commands::session::session_write,
            commands::session::session_resize,
            commands::session::session_close,
            commands::session::session_ping,
            commands::session::session_replay,
            commands::session::session_claim,
            commands::session::session_ki_respond,
            commands::session::session_log_status,
            commands::session::session_log_toggle,
            commands::session::ssh_agent_identities,
            commands::session::session_hostkey_respond,
            commands::session::knownhosts_list,
            commands::session::knownhosts_forget,
            commands::session::knownhosts_import,
            commands::session::serial_ports,
            commands::session::session_open_serial,
            commands::session::serial_send_break,
            commands::session::serial_set_signal,
            commands::session::session_open_tcp,
            commands::session::telnet_command,
            commands::session::tunnel_list_status,
            commands::session::tunnel_open,
            commands::session::tunnel_close,
            commands::files::localfs_home,
            commands::files::localfs_parent,
            commands::files::localfs_list,
            commands::files::localfs_copy_into,
            commands::files::sftp_list,
            commands::files::sftp_mkdir,
            commands::files::sftp_compare,
            commands::files::sftp_remove,
            commands::files::sftp_rename,
            commands::files::sftp_chmod,
            commands::files::sftp_preview,
            commands::files::sftp_read_file,
            commands::files::sftp_write_file,
            commands::files::sftp_upload_paths,
            commands::files::sftp_download_to,
            commands::files::sftp_drag_out,
            commands::files::sftp_name_conflicts,
            commands::files::sftp_edit,
            commands::files::sftp_edit_stop,
            commands::files::sftp_cancel_transfer,
            commands::files::sftp_pause_transfer,
            commands::files::sftp_resume_transfer,
            commands::docker::docker_list,
            commands::docker::docker_action,
            commands::docker::docker_logs,
            commands::docker::docker_stats,
            commands::docker::docker_stats_all,
            commands::docker::docker_logs_cancel,
            commands::docker::docker_container_files,
            commands::docker::docker_compose_list,
            commands::docker::docker_compose_ps,
            commands::docker::docker_compose_action,
            commands::docker::docker_compose_read,
            commands::docker::docker_compose_logs,
            commands::docker::docker_compose_logs_cancel,
            commands::db::db_open,
            commands::db::db_query,
            commands::db::db_close,
            commands::db::db_current,
            commands::db::db_cancel,
            commands::desktop::vnc_open,
            commands::desktop::vnc_pointer,
            commands::desktop::vnc_key,
            commands::desktop::vnc_refresh,
            commands::desktop::vnc_paste,
            commands::desktop::vnc_close,
            commands::desktop::rdp_open,
            commands::desktop::rdp_pointer,
            commands::desktop::rdp_key,
            commands::desktop::rdp_wheel,
            commands::desktop::rdp_secure_attention,
            commands::desktop::rdp_resize,
            commands::desktop::rdp_close,
            commands::desktop::rdp_capture,
            commands::desktop::rdp_note,
            commands::desktop::rdp_attach,
            commands::desktop::desktop_active,
            commands::desktop::vnc_attach,
            commands::desktop::desktop_rdp_detect,
            commands::desktop::desktop_rdp_install,
            commands::desktop::desktop_rdp_start,
            commands::desktop::desktop_detect,
            commands::desktop::desktop_install,
            commands::desktop::desktop_set_password,
            commands::host::workspace_processes,
            commands::host::workspace_kill,
            commands::host::workspace_services,
            commands::host::workspace_service_action,
            commands::host::workspace_logs,
            commands::host::workspace_platform,
            commands::tasks::tasks_list,
            commands::tasks::tasks_save,
            commands::tasks::tasks_delete,
            commands::tasks::tasks_export,
            commands::tasks::tasks_import,
            commands::tasks::task_runs_list,
            commands::tasks::tasks_run,
            commands::tasks::tasks_cancel,
            commands::fleet::multi_exec,
            commands::fleet::multi_exec_cancel,
            commands::observability::session_metrics_history,
            commands::observability::session_health_thresholds,
            commands::observability::session_monitor,
            commands::observability::session_sysinfo,
            commands::observability::action_log_list,
            commands::observability::action_log_verify,
            commands::observability::action_log_export,
            commands::observability::action_log_status,
            commands::observability::policy_status,
            commands::tools::tools_port_test,
            commands::tools::tools_dns_lookup,
            commands::tools::tools_tls_cert,
            commands::tools::tools_subnet,
            commands::tools::tools_hash,
            commands::tools::tools_jwt_decode,
            commands::tools::tools_port_test_on,
            commands::tools::tools_dns_lookup_on,
            commands::tools::tools_port_scan,
            commands::tools::tools_port_scan_on,
            commands::tools::tools_trace,
            commands::tools::tools_trace_on,
            commands::tools::tools_http,
            commands::tools::tools_http_on,
            commands::tools::tools_ldap,
            commands::tools::tools_diff
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
