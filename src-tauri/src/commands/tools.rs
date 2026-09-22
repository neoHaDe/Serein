//! Сетевые утилиты: порты, DNS, TLS, HTTP, трассировка, LDAP, сравнение, подсеть, хеши, JWT.

use crate::{filediff, ldap, platform, policy, remote_fs, ssh, tools, AppState};
use serde_json::{json, Value};
use tauri::State;

#[tauri::command]
pub async fn tools_port_test(host: String, port: u16, timeout_ms: Option<u64>) -> Result<Value, String> {
    policy::check_target(&host, "проверка порта")?;
    tools::port_test(host, port, timeout_ms).await
}

#[tauri::command]
pub async fn tools_dns_lookup(name: String) -> Result<Value, String> {
    policy::check_target(name.trim().trim_end_matches('.'), "запрос DNS")?;
    tools::dns_lookup(name).await
}

#[tauri::command]
pub async fn tools_tls_cert(host: String, port: Option<u16>) -> Result<Value, String> {
    policy::check_target(&host, "сертификат TLS")?;
    tools::tls_cert(host, port).await
}

/// HTTP-запрос со своей машины: код ответа, заголовки, время и цепочка переходов.
#[tauri::command]
pub async fn tools_http(url: String, method: Option<String>, max_redirects: Option<u8>) -> Result<Value, String> {
    policy::check_target(&tools::parse_url(&url)?.host, "запрос HTTP")?;
    tools::http_probe(url, method, max_redirects).await
}

/// HTTP-запрос **с сервера**: отвечает ли служба именно ему.
#[tauri::command]
pub async fn tools_http_on(
    state: State<'_, AppState>,
    session_id: String,
    url: String,
    method: Option<String>,
) -> Result<Value, String> {
    // Адрес уходит в командную строку, поэтому проверяем его тем же разбором, что и для
    // своей стороны: узел через `check_host`, схема - только http и https.
    let u = tools::parse_url(&url)?;
    policy::check_target(&u.host, "запрос HTTP с сервера")?;
    let method = method.unwrap_or_else(|| "GET".into()).to_uppercase();
    if !matches!(method.as_str(), "GET" | "HEAD") {
        return Err("Пока умеем только GET и HEAD".into());
    }
    let целый = format!(
        "{}://{}:{}{}",
        if u.secure { "https" } else { "http" },
        u.host,
        u.port,
        u.path
    );
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    if kind == platform::Kind::Windows {
        return Err("HTTP-запрос с Windows-сервера пока не поддержан".into());
    }
    let cmd = tools::remote::http_cmd_posix(&целый, &method, 10);
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(tools::remote::parse_http(&целый, &out))
}

/// Откуда брать файл для сравнения.
///
/// Смысл утилиты именно в разнородности сторон: сравнить конфиг на двух серверах или
/// локальную правку с тем, что доехало, - вопросы, которые задают чаще всего, и ни один
/// из них не решается сравнением двух файлов на одной машине.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSide {
    /// Пусто - файл на этой машине; иначе идентификатор открытой SSH-сессии.
    #[serde(default)]
    session_id: Option<String>,
    path: String,
}

impl DiffSide {
    /// Подпись стороны для показа: путь и, если это сервер, откуда он.
    fn label(&self) -> String {
        match self.session_id.as_deref().filter(|s| !s.is_empty()) {
            Some(_) => format!("сервер: {}", self.path),
            None => format!("эта машина: {}", self.path),
        }
    }
}

async fn diff_side_text(state: &State<'_, AppState>, side: &DiffSide) -> Result<String, String> {
    match side.session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => {
            let s = state.ssh(id).ok_or("Сессия не подключена")?;
            let v = remote_fs::read_file(&s.remote_fs, &s.handle, &side.path).await?;
            // Слишком большой файл читалка отдаёт с пометкой и без содержимого. Сравнивать
            // обрезанное значило бы показать различия, которых в файлах нет.
            if v.get("tooLarge").and_then(|b| b.as_bool()).unwrap_or(false) {
                return Err(format!("Файл {} слишком большой для сравнения", side.path));
            }
            Ok(v.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string())
        }
        None => tokio::fs::read_to_string(&side.path)
            .await
            .map_err(|e| format!("Не удалось прочитать {}: {e}", side.path)),
    }
}

/// Сравнение двух файлов. Каждая сторона - эта машина или любая открытая сессия.
#[tauri::command]
pub async fn tools_diff(state: State<'_, AppState>, a: DiffSide, b: DiffSide) -> Result<Value, String> {
    let (ta, tb) = (diff_side_text(&state, &a).await?, diff_side_text(&state, &b).await?);
    // Двоичные файлы не сравниваем построчно: получился бы мусор, не отвечающий ни на
    // один вопрос. Но сказать, совпадают ли они, всё равно можем.
    if filediff::looks_binary(&ta) || filediff::looks_binary(&tb) {
        return Ok(json!({
            "a": a.label(),
            "b": b.label(),
            "same": ta == tb,
            "binary": true,
            "note": "Похоже на двоичные файлы - построчное сравнение для них бессмысленно",
        }));
    }
    Ok(filediff::compare(&a.label(), &ta, &b.label(), &tb))
}

/// Запрос к каталогу LDAP.
///
/// Только со своей машины: варианта «с сервера» здесь нет, и это осознанно. LDAP - это
/// ASN.1, готовый клиент открытый поток не принимает, а писать разбор протокола ради
/// второго варианта несоразмерно пользе. В интерфейсе об этом сказано прямо.
#[tauri::command]
pub async fn tools_ldap(params: ldap::Params) -> Result<Value, String> {
    ldap::check_url(&params.url)?;
    policy::check_target(&ldap::host_of(&params.url), "каталог LDAP")?;
    ldap::search(params).await
}

/// Маршрут до адреса со своей машины.
#[tauri::command]
pub async fn tools_trace(host: String, hops: Option<u8>) -> Result<Value, String> {
    policy::check_target(&host, "трассировка")?;
    tools::trace(host, hops).await
}

/// Маршрут до адреса **с сервера**: у него свои маршруты, и это как раз тот случай,
/// когда ответ со своей машины ничего не говорит о чужой.
#[tauri::command]
pub async fn tools_trace_on(
    state: State<'_, AppState>,
    session_id: String,
    host: String,
    hops: Option<u8>,
) -> Result<Value, String> {
    let (host, _) = tools::parse_host_port(&host, 0)?;
    tools::remote::check_host(&host)?;
    policy::check_target(&host, "утилита с сервера")?;
    let hops = hops.unwrap_or(15).clamp(1, 30);
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = match kind {
        platform::Kind::Windows => tools::remote::trace_cmd_windows(&host, hops),
        _ => tools::remote::trace_cmd_posix(&host, hops),
    };
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(tools::remote::parse_trace(&host, &out))
}

/// Просмотр диапазона портов со своей машины.
#[tauri::command]
pub async fn tools_port_scan(host: String, from: u16, to: u16, timeout_ms: Option<u64>) -> Result<Value, String> {
    policy::check_target(&host, "просмотр диапазона портов")?;
    tools::port_scan(host, from, to, timeout_ms).await
}

/// Просмотр диапазона портов **с сервера**.
///
/// Проверки там идут по очереди, поэтому диапазон стоит держать узким: сотня портов на
/// недоступном хосте с секундным таймаутом - это полторы минуты ожидания.
#[tauri::command]
pub async fn tools_port_scan_on(
    state: State<'_, AppState>,
    session_id: String,
    host: String,
    from: u16,
    to: u16,
) -> Result<Value, String> {
    let (host, _) = tools::parse_host_port(&host, from)?;
    tools::remote::check_host(&host)?;
    policy::check_target(&host, "утилита с сервера")?;
    let (from, to) = tools::parse_range(from, to)?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    if kind == platform::Kind::Windows {
        return Err("Просмотр диапазона портов с Windows-сервера пока не поддержан".into());
    }
    let cmd = tools::remote::scan_cmd_posix(&host, from, to, 1);
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(tools::remote::parse_scan(&host, from, to, &out))
}

/// Проверка порта **с сервера**, а не со своей машины.
///
/// Разница не косметическая: при разборе неполадки почти всегда важно, видит ли адрес
/// сам сервер, а не тот, кто на него смотрит. Набор утилит на серверах разный, поэтому
/// команда собирается под систему, а её ответ разбирается отдельно и под тестами.
#[tauri::command]
pub async fn tools_port_test_on(
    state: State<'_, AppState>,
    session_id: String,
    host: String,
    port: u16,
) -> Result<Value, String> {
    let (host, port) = tools::parse_host_port(&host, port)?;
    tools::remote::check_host(&host)?;
    policy::check_target(&host, "утилита с сервера")?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = match kind {
        platform::Kind::Windows => tools::remote::port_cmd_windows(&host, port),
        _ => tools::remote::port_cmd_posix(&host, port, 3),
    };
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(tools::remote::parse_port(&host, port, &out))
}

/// Разрешение имени **с сервера**: у него свои DNS и свой `/etc/hosts`.
#[tauri::command]
pub async fn tools_dns_lookup_on(
    state: State<'_, AppState>,
    session_id: String,
    name: String,
) -> Result<Value, String> {
    let name = name.trim().trim_end_matches('.').to_string();
    tools::remote::check_host(&name)?;
    policy::check_target(&name, "запрос DNS с сервера")?;
    let s = state.ssh(&session_id).ok_or("Сессия не подключена")?;
    let (kind, _) = platform::of_session(&session_id, &s.handle).await;
    let cmd = match kind {
        platform::Kind::Windows => tools::remote::dns_cmd_windows(&name),
        _ => tools::remote::dns_cmd_posix(&name),
    };
    let (_c, out, _e) = ssh::exec(&s.handle, &cmd, Some(s.cancel.subscribe())).await?;
    Ok(tools::remote::parse_dns(&name, &out))
}

#[tauri::command]
pub fn tools_subnet(input: String) -> Result<Value, String> {
    tools::subnet_calc(&input)
}

#[tauri::command]
pub fn tools_hash(algo: String, text: String) -> Result<Value, String> {
    tools::hash_text(&algo, &text)
}

#[tauri::command]
pub fn tools_jwt_decode(token: String) -> Result<Value, String> {
    tools::jwt_decode(&token)
}
