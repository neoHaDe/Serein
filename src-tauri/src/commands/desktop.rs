//! Удалённый рабочий стол: VNC и RDP, подготовка сервера, окна передачи.

use crate::{actionlog, deskout, platform, policy, rdp, rdp_capture, rdpsetup, ssh, vnc, vncsetup, AppState};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

/// Что на сервере есть для рабочего стола: программа, порты, среда, права.
///
/// Спрашивается перед подключением, а не после неудачи. Раньше про отсутствие VNC можно
/// было узнать только по невнятной ошибке соединения - а это три разные беды с тремя
/// разными ответами: программы нет, программа не запущена, запущена не там.
#[tauri::command]
pub async fn desktop_detect(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    if kind == platform::Kind::Windows {
        return Ok(json!({
            "installed": [], "listening": [], "canInstall": false,
            "summary": "VNC на Windows не ставим - там есть встроенный RDP, смотри соседнюю плитку",
        }));
    }
    let (_c, out, _e) = ssh::exec(&s.handle, vncsetup::DETECT_CMD, Some(s.cancel.subscribe())).await?;
    Ok(vncsetup::parse_detect(&out))
}

/// Ставит сервер VNC на сервер.
///
/// Пароль `sudo` уходит на стандартный ввод, а не в строку команды: она целиком видна в
/// списке процессов сервера любому, кто там есть. Нигде не сохраняется.
#[tauri::command]
pub async fn desktop_install(
    state: State<'_, AppState>,
    session_id: String,
    package_manager: String,
    sudo_password: String,
) -> Result<Value, String> {
    let cmd = vncsetup::install_cmd(&package_manager).ok_or("Не знаем, как ставить пакеты этим менеджером")?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (code, out, err) = ssh::exec_with_input(
        &s.handle,
        &cmd,
        &format!("{sudo_password}\n"),
        Some(s.cancel.subscribe()),
    )
    .await?;
    if code != 0 {
        // Неверный пароль sudo выглядит именно так, и сказать об этом прямо полезнее,
        // чем показать сырой вывод пакетного менеджера.
        let текст = if out.contains("incorrect password") || err.contains("incorrect password") {
            "Пароль sudo не подошёл".to_string()
        } else {
            let x = format!("{out}\n{err}");
            let x = x.trim();
            if x.is_empty() {
                format!("установка вернула код {code}")
            } else {
                x.to_string()
            }
        };
        return Ok(json!({ "ok": false, "error": текст }));
    }
    Ok(json!({ "ok": true }))
}

/// Задаёт пароль рабочего стола.
///
/// Выполняется от самого пользователя, без sudo: файл пароля лежит в его домашнем каталоге.
#[tauri::command]
pub async fn desktop_set_password(
    state: State<'_, AppState>,
    session_id: String,
    password: String,
) -> Result<Value, String> {
    vncsetup::check_vnc_password(&password)?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (code, out, err) = ssh::exec_with_input(
        &s.handle,
        vncsetup::SET_PASSWORD_CMD,
        &format!("{password}\n"),
        Some(s.cancel.subscribe()),
    )
    .await?;
    if code != 0 || !out.contains("OK") {
        // Сюда попадает то, что сказала сама программа. Раньше её вывод глушился, и панель
        // показывала «не удалось сохранить пароль» без единого слова о причине - на сервере
        // с tigervnc это выглядело как поломка на пустом месте.
        let x = format!("{err}\n{out}");
        let x = x.trim();
        return Ok(json!({
            "ok": false,
            "error": if x.is_empty() { format!("команда вернула код {code}") } else { x.to_string() },
        }));
    }
    Ok(json!({ "ok": true }))
}

/// Окно SSH-соединения рабочего стола - сколько сервер держит в пути, не дожидаясь нас.
///
/// Ровно столько же и может встать в очередь перед узким местом сети, поэтому для VPN
/// оно маленькое: 128 КиБ на канале в 10 Мбит/с - это не больше 0,1 с задержки сверху,
/// а потолок скорости при задержке 50 мс - около 20 Мбит/с, картинке этого хватает.
/// В локальной сети задержка ничтожна, и то же окно дало бы лишь потолок скорости.
const DESKTOP_WINDOW_VPN: u32 = 128 * 1024;

const DESKTOP_WINDOW_LAN: u32 = 1024 * 1024;

/// VNC сам запрашивает каждый кадр и лишнего не шлёт - окно чуть свободнее.
const DESKTOP_WINDOW_VNC: u32 = 256 * 1024;

/// Закрывает рабочий стол этой сессии, если он открыт.
///
/// Один стол на сессию - так устроена и панель. Но полагаться на панель здесь нельзя:
/// второе открытие подряд перезаписывало запись о живом столе, и первый оставался в
/// памяти со своим процессом и каналом, никому не известный. Закрыть его потом было
/// нечем: ни панель, ни закрытие сессии его уже не находили.
pub(crate) fn close_desktop_of(session_id: &str) {
    if let Some(desk) = deskout::active(session_id) {
        match desk.kind {
            deskout::Kind::Rdp => rdp::close(&desk.id),
            deskout::Kind::Vnc => vnc::close(&desk.id),
        }
    }
}

/// Своё SSH-соединение под рабочий стол, а если не вышло - общий канал сессии.
///
/// «Не вышло» - не ошибка: второй фактор при входе, пароль, который не сохранён, быстрое
/// подключение без профиля. Рабочий стол тогда работает как раньше, общим каналом, а
/// причина уходит в журнал: без неё вопрос «почему терминал тормозит, пока открыт
/// рабочий стол» не разобрать.
async fn desktop_link(server_id: &str, window: u32, compress: bool) -> Option<ssh::DesktopLink> {
    if server_id.is_empty() {
        rdp::log("рабочий стол идёт общим каналом сессии: у неё нет сохранённого профиля");
        return None;
    }
    let chain = match crate::chain::resolve(server_id) {
        Ok(c) => c,
        Err(e) => {
            rdp::log(&format!("рабочий стол идёт общим каналом сессии: {e}"));
            return None;
        }
    };
    let attempt = ssh::connect_desktop(chain, window, compress);
    match tokio::time::timeout(std::time::Duration::from_secs(10), attempt).await {
        Ok(Ok(link)) => {
            rdp::log(&format!(
                "рабочий стол: своё SSH-соединение, окно {} КиБ{}",
                window / 1024,
                if compress { ", сжатие zlib" } else { "" }
            ));
            Some(link)
        }
        Ok(Err(e)) => {
            rdp::log(&format!(
                "рабочий стол идёт общим каналом сессии: своё соединение не поднялось: {e}"
            ));
            None
        }
        Err(_) => {
            rdp::log("рабочий стол идёт общим каналом сессии: своё соединение не поднялось за 10 с");
            None
        }
    }
}

/// Открывает рабочий стол VNC поверх уже подключённой SSH-сессии.
///
/// Через сессию, а не напрямую, потому что VNC на сервере почти всегда слушает `127.0.0.1`
/// и наружу не смотрит - и правильно делает: свой протокол он защищает паролем до восьми
/// символов на DES. Ходить к нему нужно внутри SSH, а не открывать порт в сеть.
#[tauri::command]
pub async fn vnc_open(
    state: State<'_, AppState>,
    session_id: String,
    host: Option<String>,
    port: Option<u16>,
    password: Option<String>,
    on_frame: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<String, vnc::OpenError> {
    let s = state
        .ssh(&session_id)
        .ok_or_else(|| vnc::OpenError::from("Сессия не подключена".to_string()))?;
    close_desktop_of(&session_id);
    let id = format!("vnc-{}", uuid::Uuid::new_v4());
    // Tight и ZRLE уже сжаты zlib: второй раз сжимать их на уровне SSH - пустая работа.
    let link = desktop_link(&s.server_id, DESKTOP_WINDOW_VNC, false).await;
    policy::check_target(host.as_deref().unwrap_or("127.0.0.1"), "рабочий стол VNC").map_err(vnc::OpenError::from)?;
    let journal = json!({ "host": host.as_deref().unwrap_or("127.0.0.1"), "port": port.unwrap_or(5900) });
    let target = vnc::Target::Ssh {
        handle: link.as_ref().map_or_else(|| s.handle.clone(), |l| l.handle.clone()),
        host: host.unwrap_or_else(|| "127.0.0.1".into()),
        // 5900 - нулевой дисплей; у большинства серверов рабочий стол именно там.
        port: port.unwrap_or(5900),
        link,
    };
    let r = vnc::open(id.clone(), session_id.clone(), target, password, on_frame).await;
    actionlog::record_session(
        &session_id,
        "vnc.open",
        journal,
        &r.as_ref().map(|_| ()).map_err(|e| e.message.clone()),
    );
    r?;
    Ok(id)
}

/// Движение мыши и нажатия. Кнопки - битовой маской, как в RFB: 1 левая, 2 средняя,
/// 4 правая, 8 и 16 - колесо вверх и вниз.
#[tauri::command]
pub fn vnc_pointer(id: String, x: u16, y: u16, buttons: u8) {
    vnc::input(&id, vnc::X11Event::PointerEvent((x, y, buttons).into()));
}

#[tauri::command]
pub fn vnc_key(id: String, keysym: u32, down: bool) {
    vnc::input(&id, vnc::X11Event::KeyEvent((keysym, down).into()));
}

/// Запрос обновления экрана. Полное обновление нужно после переподключения или когда
/// картинка «поехала»: сервер шлёт только изменения и сам себя не перерисовывает.
#[tauri::command]
pub fn vnc_refresh(id: String, full: bool) {
    vnc::input(
        &id,
        if full {
            vnc::X11Event::FullRefresh
        } else {
            vnc::X11Event::Refresh
        },
    );
}

/// Вставка на удалённый рабочий стол.
///
/// В RFB буфер обмена и вставка - разные вещи: `ClientCutText` только кладёт текст в буфер
/// сервера, но никуда его не вставляет. Поэтому следом отправляется Shift+Insert - это
/// сочетание понимают и xterm, и обычные приложения X, в отличие от Ctrl+V, который в
/// терминалах не работает.
#[tauri::command]
pub fn vnc_paste(id: String, text: String) {
    vnc::paste(&id, text);
}

#[tauri::command]
pub fn vnc_close(id: String) {
    vnc::close(&id);
}

/// Есть ли у этой SSH-сессии уже открытый рабочий стол.
///
/// Спрашивается при открытии панели. Нужно для откреплённого окна: сеанс живёт в
/// приложении, а не в окне, и второе окно должно продолжить картинку, а не начинать с
/// ввода пароля - тот же сеанс, тот же сервер, зачем спрашивать дважды.
#[tauri::command]
pub fn desktop_active(session_id: String) -> Option<Value> {
    deskout::active(&session_id)
        .map(|a| json!({ "kind": a.kind.as_str(), "id": a.id, "width": a.size.0, "height": a.size.1 }))
}

/// Переводит выдачу кадров VNC в это окно и просит перерисовать экран целиком.
#[tauri::command]
pub fn vnc_attach(id: String, on_frame: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>) -> Result<(), String> {
    vnc::attach(&id, on_frame)
}

/// То же для RDP.
#[tauri::command]
pub fn rdp_attach(id: String, on_frame: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>) -> Result<(), String> {
    rdp::attach(&id, on_frame)
}

/// Что на сервере есть для RDP: программа, порт, служба, права.
///
/// Отдельно от разведки VNC: смотреть надо другое, а лишний вопрос серверу дешевле, чем
/// одна команда, отвечающая сразу за двоих и путающая оба ответа.
#[tauri::command]
pub async fn desktop_rdp_detect(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    if kind == platform::Kind::Windows {
        // На Windows RDP встроенный, ставить нечего - но он бывает выключен, и панель
        // должна сказать, чем именно: настройками, службой или межсетевым экраном.
        let (_c, out, _e) = ssh::exec(
            &s.handle,
            &platform::ps(rdpsetup::DETECT_WINDOWS),
            Some(s.cancel.subscribe()),
        )
        .await?;
        return Ok(rdpsetup::parse_detect_windows(&out));
    }
    let (_c, out, _e) = ssh::exec(&s.handle, rdpsetup::DETECT_CMD, Some(s.cancel.subscribe())).await?;
    Ok(rdpsetup::parse_detect(&out))
}

/// Ставит xrdp на сервер.
#[tauri::command]
pub async fn desktop_rdp_install(
    state: State<'_, AppState>,
    session_id: String,
    package_manager: String,
    sudo_password: String,
) -> Result<Value, String> {
    let cmd = rdpsetup::install_cmd(&package_manager)
        .ok_or("Этим менеджером пакетов xrdp не поставить: пакета нет в основных хранилищах")?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    run_setup_step(&s, &cmd, &sudo_password, "установка").await
}

/// Включает и запускает службу xrdp.
///
/// Именно включает, а не только запускает: иначе после перезагрузки сервера рабочий стол
/// молча не поднимется, и выяснится это в самый неудачный момент.
#[tauri::command]
pub async fn desktop_rdp_start(
    state: State<'_, AppState>,
    session_id: String,
    sudo_password: String,
    open_firewall: Option<bool>,
) -> Result<Value, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    // На Windows ставить нечего: снимаем запрет и поднимаем службу. Пароль sudo там не при
    // чём - нужны права администратора самой сессии.
    //
    // Межсетевой экран открывается только по отдельной просьбе. Serein ходит к рабочему
    // столу каналом внутри SSH-сессии, на петлю самого сервера, и входящий доступ из сети
    // для этого не нужен: открывать его «за компанию» значит выставить порт 3389 наружу
    // молча.
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    if kind == platform::Kind::Windows {
        let (_c, out, _e) = ssh::exec(
            &s.handle,
            &platform::ps(&rdpsetup::enable_windows(open_firewall.unwrap_or(false))),
            Some(s.cancel.subscribe()),
        )
        .await?;
        return Ok(rdpsetup::parse_enable_windows(&out));
    }
    let r = run_setup_step(&s, rdpsetup::ENABLE_CMD, &sudo_password, "запуск").await?;
    if r["ok"] != true {
        return Ok(r);
    }
    // `systemctl` возвращает ноль, успев только отправить запрос. Служба, упавшая
    // секундой позже, ответила бы «готово» - поэтому спрашиваем её саму.
    let out = r["output"].as_str().unwrap_or_default();
    if out.lines().any(|l| l.trim() == "active") {
        Ok(json!({ "ok": true }))
    } else {
        Ok(json!({
            "ok": false,
            "error": format!("служба не поднялась: {}", out.trim()),
        }))
    }
}

/// Общая часть установки и запуска: выполнить с паролем на входе и разобрать отказ.
///
/// Неверный пароль sudo выглядит одинаково в обоих случаях, и сказать об этом прямо
/// полезнее, чем показать сырой вывод команды.
async fn run_setup_step(
    s: &std::sync::Arc<ssh::SshSession>,
    cmd: &str,
    sudo_password: &str,
    что: &str,
) -> Result<Value, String> {
    let (code, out, err) = ssh::exec_with_input(
        &s.handle,
        cmd,
        &format!("{sudo_password}\n"),
        Some(s.cancel.subscribe()),
    )
    .await?;
    if code != 0 {
        let текст = if out.contains("incorrect password") || err.contains("incorrect password") {
            "Пароль sudo не подошёл".to_string()
        } else {
            let x = format!("{out}\n{err}");
            let x = x.trim();
            if x.is_empty() {
                format!("{что} вернулась с кодом {code}")
            } else {
                x.to_string()
            }
        };
        return Ok(json!({ "ok": false, "error": текст }));
    }
    Ok(json!({ "ok": true, "output": out }))
}

/// Открывает рабочий стол по RDP.
///
/// Протокол разбирает отдельный процесс, а не этот модуль: зависимости IronRDP не
/// сходятся с SSH-ядром в одном дереве, подробности в `rdp.rs`. Приложение здесь держит
/// канал внутри SSH-сессии и подставляет его помощнику локальным сокетом.
#[tauri::command]
pub async fn rdp_open(
    state: State<'_, AppState>,
    session_id: String,
    opts: rdp::OpenRequest,
    on_frame: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<String, String> {
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    // Прежний стол этой сессии закрываем сами, а не надеемся на панель.
    close_desktop_of(&session_id);
    let req = opts.resolve()?;
    // Политику - до отдельного SSH-соединения стола: поднимать его ради отказа незачем.
    policy::check_target(&req.host, "рабочий стол RDP")?;
    let id = format!("rdp-{}", uuid::Uuid::new_v4());
    // Для медленного канала окно меньше и поток сжимается на уровне SSH: сжатие внутри
    // RDP (MPPC) ломает пересогласование после смены размера, подробности в помощнике.
    let link = match req.options.network_profile {
        rdp::NetworkProfile::Vpn => desktop_link(&s.server_id, DESKTOP_WINDOW_VPN, true).await,
        rdp::NetworkProfile::Lan => desktop_link(&s.server_id, DESKTOP_WINDOW_LAN, false).await,
    };
    let journal = req.journal();
    let target = rdp::Target::Ssh {
        handle: link.as_ref().map_or_else(|| s.handle.clone(), |l| l.handle.clone()),
        host: req.host,
        port: req.port,
        link,
    };
    let r = rdp::open(
        id.clone(),
        session_id.clone(),
        target,
        rdp::Login {
            user: req.user,
            password: req.password,
            domain: req.domain,
        },
        req.size,
        req.options,
        on_frame,
    )
    .await;
    actionlog::record_session(&session_id, "rdp.open", journal, &r);
    r?;
    Ok(id)
}

#[tauri::command]
pub fn rdp_pointer(id: String, x: u16, y: u16, buttons: u8) {
    rdp::pointer(&id, x, y, buttons);
}

#[tauri::command]
pub fn rdp_key(id: String, code: u16, down: bool) {
    rdp::key(&id, code, down);
}

#[tauri::command]
pub fn rdp_wheel(id: String, vertical: bool, delta: i16) {
    rdp::wheel(&id, vertical, delta);
}

#[tauri::command]
pub fn rdp_secure_attention(id: String) {
    rdp::secure_attention(&id);
}

/// Меняет размер рабочего стола в уже открытом сеансе.
#[tauri::command]
pub fn rdp_resize(id: String, width: u16, height: u16) {
    rdp::resize(&id, width, height);
}

#[tauri::command]
pub fn rdp_close(id: String) {
    rdp::close(&id);
}

/// Перехват системных сочетаний Windows для сеанса. `true` - хук стоит; `false` - сочетания
/// ловит только само окно, и Win с Alt+Tab остаются системе.
#[tauri::command]
pub fn rdp_capture(app: AppHandle, window: tauri::WebviewWindow, id: String, on: bool) -> bool {
    if !on {
        rdp_capture::stop(Some(&id));
        return false;
    }
    #[cfg(windows)]
    let hwnd = match window.hwnd() {
        Ok(h) => h.0 as isize,
        Err(e) => {
            rdp::note(&format!("перехват сочетаний не включён: нет окна: {e}"));
            return false;
        }
    };
    #[cfg(not(windows))]
    let hwnd = {
        let _ = &window;
        0
    };
    match rdp_capture::start(app, id, hwnd) {
        Ok(()) => true,
        Err(e) => {
            rdp::note(&format!("перехват сочетаний не включён: {e}"));
            false
        }
    }
}

/// Отчёт интерфейса о своей половине пути кадра.
#[tauri::command]
pub fn rdp_note(line: String) {
    rdp::note(&line);
}
