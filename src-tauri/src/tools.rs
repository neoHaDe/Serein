//! Утилиты: порт, DNS, TLS, подсеть, хеши, JWT (P2.1).
//!
//! Часть из них умеет отвечать на два разных вопроса. «Доступен ли адрес **с моей
//! машины**» и «доступен ли он **с сервера**» — это не одно и то же, и при разборе
//! неполадки почти всегда нужен второй. У конкурентов утилиты работают только с машины
//! пользователя; у нас уже открыта SSH-сессия, и спросить сервер стоит одного `exec`.
//!
//! Команды для сервера собираются в [`remote`], ответы разбираются там же и под тестами:
//! набор утилит на живых машинах разный, и это не мелочь. На голом Debian нет `nc`, зато
//! есть `bash` с его `/dev/tcp`; на Alpine ровно наоборот.

use base64::{engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}, Engine};
use native_tls::{TlsConnector, TlsStream};
use serde_json::{json, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;
use x509_parser::prelude::*;

const DEFAULT_PORT_TIMEOUT_MS: u64 = 3000;

pub fn parse_host_port(input: &str, default_port: u16) -> Result<(String, u16), String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Пустой хост".into());
    }
    if input.starts_with('[') {
        let end = input.find(']').ok_or("Неверный IPv6-адрес")?;
        let host = input[1..end].to_string();
        let port = if input.len() > end + 1 {
            let rest = &input[end + 1..];
            if !rest.starts_with(':') {
                return Err("Ожидался «]:port» для IPv6".into());
            }
            rest[1..].parse().map_err(|_| "Неверный порт".to_string())?
        } else {
            default_port
        };
        return Ok((host, port));
    }
    let colon_count = input.matches(':').count();
    if colon_count == 1 {
        let (host, port_s) = input.split_once(':').unwrap();
        if host.is_empty() {
            return Err("Пустой хост".into());
        }
        let port: u16 = port_s.parse().map_err(|_| "Неверный порт".to_string())?;
        return Ok((host.to_string(), port));
    }
    Ok((input.to_string(), default_port))
}

pub async fn port_test(host: String, port: u16, timeout_ms: Option<u64>) -> Result<Value, String> {
    let (host, port) = parse_host_port(&host, port)?;
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_PORT_TIMEOUT_MS).clamp(200, 60_000));
    let addr = format!("{host}:{port}");
    let started = std::time::Instant::now();
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => Ok(json!({
            "ok": true,
            "host": host,
            "port": port,
            "latencyMs": started.elapsed().as_millis(),
        })),
        Ok(Err(e)) => Ok(json!({
            "ok": false,
            "host": host,
            "port": port,
            "error": e.to_string(),
        })),
        Err(_) => {
            let ms = timeout.as_millis();
            Ok(json!({
                "ok": false,
                "host": host,
                "port": port,
                "error": format!("Таймаут {ms} мс"),
            }))
        }
    }
}

/// Сколько портов позволено просмотреть за один раз.
///
/// Ограничение не техническое, а по смыслу. Скан на тысячи портов — это уже другая задача
/// и другой инструмент (nmap), а здесь он занял бы минуты и выглядел бы как зависшее окно.
/// Полный диапазон в 65 тысяч портов через SSH-канал не осилит никакая панель.
pub const MAX_SCAN_PORTS: u32 = 1024;

/// Разбор и проверка диапазона портов.
pub fn parse_range(from: u16, to: u16) -> Result<(u16, u16), String> {
    if from == 0 || to == 0 {
        return Err("Порт 0 не существует".into());
    }
    if from > to {
        return Err("Начало диапазона больше конца".into());
    }
    let count = to as u32 - from as u32 + 1;
    if count > MAX_SCAN_PORTS {
        return Err(format!(
            "За раз можно просмотреть не больше {MAX_SCAN_PORTS} портов, а тут {count}"
        ));
    }
    Ok((from, to))
}

/// Просмотр диапазона портов со своей машины.
///
/// Порты проверяются пачками, а не по очереди: тысяча последовательных попыток с таймаутом
/// в секунду — это шестнадцать минут, и никто столько не ждёт. Ширина пачки выбрана так,
/// чтобы не упереться в предел открытых сокетов на слабой машине.
pub async fn port_scan(
    host: String,
    from: u16,
    to: u16,
    timeout_ms: Option<u64>,
) -> Result<Value, String> {
    let (host, _) = parse_host_port(&host, from)?;
    let (from, to) = parse_range(from, to)?;
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(1000).clamp(100, 10_000));

    let started = std::time::Instant::now();
    let mut open: Vec<u16> = Vec::new();
    let mut ports = from..=to;
    loop {
        let batch: Vec<u16> = ports.by_ref().take(128).collect();
        if batch.is_empty() {
            break;
        }
        let checks = batch.into_iter().map(|port| {
            let addr = format!("{host}:{port}");
            async move {
                match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr)).await {
                    Ok(Ok(_)) => Some(port),
                    _ => None,
                }
            }
        });
        open.extend(futures::future::join_all(checks).await.into_iter().flatten());
    }
    open.sort_unstable();

    Ok(json!({
        "host": host,
        "from": from,
        "to": to,
        "open": open,
        "scanned": to as u32 - from as u32 + 1,
        "ms": started.elapsed().as_millis(),
    }))
}

/// Трассировка со своей машины.
///
/// Через системную программу, а не своими пакетами: чтобы построить маршрут самому, нужен
/// сырой сокет и управление TTL, а это права администратора на Linux и лишний повод для
/// вопросов на любой машине. `tracert` на Windows есть всегда, `traceroute` на юниксах —
/// почти всегда; если нет, скажем об этом словами.
///
/// Вывод разбирается тем же кодом, что и серверный: слова там не читаются, только номер
/// узла, адрес и время, — поэтому язык системы значения не имеет.
pub async fn trace(host: String, hops: Option<u8>) -> Result<Value, String> {
    let (host, _) = parse_host_port(&host, 0)?;
    remote::check_host(&host)?;
    let hops = hops.unwrap_or(15).clamp(1, 30);

    let (prog, args) = if cfg!(windows) {
        ("tracert", vec!["-d".into(), "-h".into(), hops.to_string(), "-w".into(), "1000".into(), host.clone()])
    } else {
        ("traceroute", vec!["-n".into(), "-m".into(), hops.to_string(), "-w".into(), "1".into(), "-q".into(), "1".into(), host.clone()])
    };

    let out = tokio::process::Command::new(prog)
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("Не удалось запустить {prog}: {e}"))?;

    // Программы пишут маршрут и в stdout, и (при отказах) в stderr — берём оба.
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&out.stderr).to_string();
    }
    let mut v = remote::parse_trace(&host, &format!("TOOL={prog}\n{text}"));
    if let Some(o) = v.as_object_mut() {
        // Пометка «с сервера» здесь неверна: это наша машина.
        o.insert("from_server".into(), json!(false));
    }
    Ok(v)
}

pub async fn dns_lookup(name: String) -> Result<Value, String> {
    let name = name.trim().trim_end_matches('.').to_string();
    if name.is_empty() {
        return Err("Пустое имя".into());
    }
    let started = std::time::Instant::now();
    let addrs: Vec<String> = tokio::net::lookup_host(format!("{name}:0"))
        .await
        .map_err(|e| format!("DNS: {e}"))?
        .map(|a: SocketAddr| a.ip().to_string())
        .collect();
    Ok(json!({
        "name": name,
        "addresses": addrs,
        "latencyMs": started.elapsed().as_millis(),
    }))
}

fn cert_summary(der: &[u8]) -> Result<Value, String> {
    let (_, cert) = X509Certificate::from_der(der).map_err(|e| format!("Сертификат: {e}"))?;
    let subject = cert.subject().to_string();
    let issuer = cert.issuer().to_string();
    let not_before = cert.validity().not_before.to_string();
    let not_after = cert.validity().not_after.to_string();
    let mut san: Vec<String> = Vec::new();
    if let Ok(Some(ext)) = cert.subject_alternative_name() {
        for gn in ext.value.general_names.iter() {
            if let GeneralName::DNSName(d) = gn {
                san.push(d.to_string());
            }
        }
    }
    let fp = hex::encode(Sha256::digest(der));
    Ok(json!({
        "subject": subject,
        "issuer": issuer,
        "notBefore": not_before,
        "notAfter": not_after,
        "sha256": fp,
        "san": san,
    }))
}

fn tls_fetch_sync(host: String, port: u16) -> Result<Value, String> {
    let addr = format!("{host}:{port}");
    let tcp = TcpStream::connect_timeout(
        &addr
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .next()
            .ok_or_else(|| format!("Не удалось разрешить «{host}»"))?,
        Duration::from_secs(10),
    )
    .map_err(|e| format!("TCP: {e}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    tcp.set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    let connector = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|e| e.to_string())?;
    let tls: TlsStream<TcpStream> = connector.connect(&host, tcp).map_err(|e| e.to_string())?;
    let peer = tls
        .peer_certificate()
        .map_err(|e| e.to_string())?
        .ok_or("Сервер не прислал сертификат")?;
    let der = peer.to_der().map_err(|e| e.to_string())?;
    let chain = vec![cert_summary(&der)?];
    Ok(json!({
        "host": host,
        "port": port,
        "certificates": chain,
    }))
}

pub async fn tls_cert(host: String, port: Option<u16>) -> Result<Value, String> {
    let (host, port) = parse_host_port(&host, port.unwrap_or(443))?;
    tokio::task::spawn_blocking(move || tls_fetch_sync(host, port))
        .await
        .map_err(|e| e.to_string())?
}

fn parse_ipv4(s: &str) -> Result<Ipv4Addr, String> {
    s.trim()
        .parse()
        .map_err(|_| format!("Неверный IPv4: «{s}»"))
}

fn mask_from_prefix(prefix: u8) -> Result<Ipv4Addr, String> {
    if prefix > 32 {
        return Err("Префикс должен быть 0–32".into());
    }
    if prefix == 0 {
        return Ok(Ipv4Addr::new(0, 0, 0, 0));
    }
    let bits = u32::MAX << (32 - prefix);
    Ok(Ipv4Addr::from(bits))
}

pub fn subnet_calc(input: &str) -> Result<Value, String> {
    let input = input.trim();
    if let Some((a, b)) = input.split_once('/') {
        let ip = parse_ipv4(a.trim())?;
        let prefix = b.trim().parse::<u8>().map_err(|_| "Неверный префикс")?;
        return subnet_from(u32::from(ip), prefix);
    }
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() == 2 {
        let ip = parse_ipv4(parts[0])?;
        let mask = parse_ipv4(parts[1])?;
        let mask_u = u32::from(mask);
        if mask_u.count_ones() == 0 || (mask_u & !mask_u.wrapping_add(1)) != 0 {
            return Err("Маска должна быть непрерывной".into());
        }
        let prefix = mask_u.count_ones() as u8;
        return subnet_from(u32::from(ip), prefix);
    }
    Err("Ожидался CIDR (10.0.0.0/24) или «IP маска»".into())
}

fn subnet_from(ip: u32, prefix: u8) -> Result<Value, String> {
    let mask = u32::from(mask_from_prefix(prefix)?);
    let network = ip & mask;
    let broadcast = network | !mask;
    let wildcard = !mask;
    let host_count = if prefix >= 31 {
        0u32
    } else {
        2u32.pow(32 - u32::from(prefix)) - 2
    };
    let first = if prefix >= 31 {
        network
    } else {
        network + 1
    };
    let last = if prefix >= 31 {
        broadcast
    } else {
        broadcast - 1
    };
    Ok(json!({
        "input": format!("{}/{}", Ipv4Addr::from(ip), prefix),
        "network": Ipv4Addr::from(network).to_string(),
        "prefix": prefix,
        "netmask": Ipv4Addr::from(mask).to_string(),
        "wildcard": Ipv4Addr::from(wildcard).to_string(),
        "broadcast": Ipv4Addr::from(broadcast).to_string(),
        "firstHost": Ipv4Addr::from(first).to_string(),
        "lastHost": Ipv4Addr::from(last).to_string(),
        "hostCount": host_count,
    }))
}

/// Калькулятор хешей для пользователя.
///
/// ⚠ MD5 и SHA-1 здесь именно как калькулятор — их просят, чтобы сверить чужую
/// контрольную сумму, и без них утилита бесполезна. Внутри продукта они не используются
/// нигде: секреты закрыты AES-256-GCM со scrypt, подпись обновлений — minisign. Строка
/// написана затем, чтобы при следующем аудите зависимостей `md5` и `sha1` не приняли
/// за криптографию Serein.
pub fn hash_text(algo: &str, text: &str) -> Result<Value, String> {
    let bytes = text.as_bytes();
    let (hex, b64) = match algo.to_ascii_lowercase().as_str() {
        "md5" => {
            let d = md5::compute(bytes);
            (hex::encode(d.0), STANDARD.encode(d.0))
        }
        "sha1" => {
            let d = Sha1::digest(bytes);
            (hex::encode(d), STANDARD.encode(d))
        }
        "sha256" => {
            let d = Sha256::digest(bytes);
            (hex::encode(d), STANDARD.encode(d))
        }
        "sha512" => {
            let d = Sha512::digest(bytes);
            (hex::encode(d), STANDARD.encode(d))
        }
        other => return Err(format!("Неизвестный алгоритм: {other}")),
    };
    Ok(json!({ "algo": algo.to_ascii_lowercase(), "hex": hex, "base64": b64 }))
}

fn b64url_json(part: &str) -> Result<Value, String> {
    let raw = URL_SAFE_NO_PAD
        .decode(part)
        .or_else(|_| URL_SAFE_NO_PAD.decode(format!("{part}=")))
        .or_else(|_| URL_SAFE_NO_PAD.decode(format!("{part}==")))
        .map_err(|e| format!("Base64: {e}"))?;
    let txt = String::from_utf8(raw).map_err(|e| format!("UTF-8: {e}"))?;
    serde_json::from_str(&txt).map_err(|e| format!("JSON: {e}"))
}

pub fn jwt_decode(token: &str) -> Result<Value, String> {
    let token = token.trim();
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return Err("JWT должен содержать минимум header.payload".into());
    }
    Ok(json!({
        "header": b64url_json(parts[0])?,
        "payload": b64url_json(parts[1])?,
        "signature": parts.get(2).unwrap_or(&""),
    }))
}

/// Проверки и команды для запуска утилит **на сервере**.
pub mod remote {
    use serde_json::{json, Value};

    /// Проверка хоста перед подстановкой в команду.
    ///
    /// Хост уходит в командную строку на чужой машине, поэтому список разрешённого узкий:
    /// буквы, цифры, точка, дефис и двоеточие для IPv6. Отказ, а не экранирование —
    /// экранирование легко сделать неполным, а короткий список проверяется взглядом.
    /// Отдельно запрещено начинать с дефиса: такое имя прочтётся как ключ команды.
    pub fn check_host(host: &str) -> Result<(), String> {
        if host.is_empty() || host.len() > 253 {
            return Err("Пустой или слишком длинный адрес".into());
        }
        if host.starts_with('-') {
            return Err("Адрес не может начинаться с дефиса".into());
        }
        if !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'))
        {
            return Err("В адресе есть символы, которых там быть не может".into());
        }
        Ok(())
    }

    /// Проверка порта на POSIX-системе.
    ///
    /// Две ветки не для надёжности ради надёжности: на голом Debian нет `nc`, а на Alpine
    /// нет `bash` — то есть ни одна из них по отдельности не покрывает даже наш стенд.
    pub fn port_cmd_posix(host: &str, port: u16, secs: u64) -> String {
        format!(
            "if command -v nc >/dev/null 2>&1; then echo TOOL=nc; \
               nc -z -w {secs} '{host}' {port} >/dev/null 2>&1 && echo R=open || echo R=closed; \
             elif command -v timeout >/dev/null 2>&1 && command -v bash >/dev/null 2>&1; then \
               echo TOOL=bash; \
               timeout {secs} bash -c 'exec 3<>/dev/tcp/{host}/{port}' >/dev/null 2>&1 \
                 && echo R=open || echo R=closed; \
             else echo TOOL=none; fi"
        )
    }

    /// То же для Windows: там ни `nc`, ни `/dev/tcp` не существует.
    pub fn port_cmd_windows(host: &str, port: u16) -> String {
        format!(
            "powershell -NoProfile -NonInteractive -Command \"echo TOOL=powershell; \
             $r = Test-NetConnection -ComputerName '{host}' -Port {port} -InformationLevel Quiet \
             -WarningAction SilentlyContinue; if ($r) {{ echo R=open }} else {{ echo R=closed }}\""
        )
    }

    /// Разбор ответа проверки порта.
    pub fn parse_port(host: &str, port: u16, stdout: &str) -> Value {
        let tool = tag(stdout, "TOOL=").unwrap_or_default();
        if tool == "none" {
            return json!({
                "ok": false,
                "host": host,
                "port": port,
                "from": "server",
                "error": "На сервере нечем проверить порт: нет ни nc, ни bash",
            });
        }
        match tag(stdout, "R=").as_deref() {
            Some("open") => json!({ "ok": true, "host": host, "port": port, "from": "server", "tool": tool }),
            Some("closed") => json!({
                "ok": false,
                "host": host,
                "port": port,
                "from": "server",
                "tool": tool,
                "error": "Порт закрыт или недоступен с сервера",
            }),
            // Ответа нет вовсе — команда не выполнилась, и выдавать это за «закрыт»
            // нельзя: закрытый порт и несостоявшаяся проверка — разные новости.
            _ => json!({
                "ok": false,
                "host": host,
                "port": port,
                "from": "server",
                "error": "Сервер не ответил на проверку",
            }),
        }
    }

    /// Просмотр диапазона портов на POSIX-системе.
    ///
    /// Проверки идут по очереди, а не пачкой: раскладывать их в фоновые процессы оболочки
    /// значит рисковать упереться в лимит процессов на чужой машине ради чужой задачи.
    /// Поэтому и таймаут здесь короче, и диапазон разумно держать узким.
    pub fn scan_cmd_posix(host: &str, from: u16, to: u16, secs: u64) -> String {
        format!(
            "if command -v nc >/dev/null 2>&1; then echo TOOL=nc; \
               p={from}; while [ $p -le {to} ]; do \
                 nc -z -w {secs} '{host}' $p >/dev/null 2>&1 && echo P=$p; \
                 p=$((p+1)); done; \
             elif command -v timeout >/dev/null 2>&1 && command -v bash >/dev/null 2>&1; then \
               echo TOOL=bash; \
               p={from}; while [ $p -le {to} ]; do \
                 timeout {secs} bash -c \"exec 3<>/dev/tcp/{host}/$p\" >/dev/null 2>&1 && echo P=$p; \
                 p=$((p+1)); done; \
             else echo TOOL=none; fi"
        )
    }

    /// Разбор ответа просмотра диапазона.
    pub fn parse_scan(host: &str, from: u16, to: u16, stdout: &str) -> Value {
        let tool = tag(stdout, "TOOL=").unwrap_or_default();
        if tool == "none" {
            return json!({
                "host": host,
                "from": from,
                "to": to,
                "from_server": true,
                "error": "На сервере нечем проверить порты: нет ни nc, ни bash",
            });
        }
        let mut open: Vec<u16> = Vec::new();
        for line in stdout.lines() {
            if let Some(p) = line.trim().strip_prefix("P=") {
                if let Ok(n) = p.trim().parse::<u16>() {
                    if (from..=to).contains(&n) && !open.contains(&n) {
                        open.push(n);
                    }
                }
            }
        }
        open.sort_unstable();
        json!({
            "host": host,
            "from": from,
            "to": to,
            "from_server": true,
            "tool": tool,
            "open": open,
            "scanned": to as u32 - from as u32 + 1,
        })
    }

    /// Трассировка на POSIX-системе.
    ///
    /// Программ для неё несколько, и есть машины, где нет ни одной: на голом Debian нет
    /// ни `traceroute`, ни `tracepath`, ни даже `ping`. Это не редкость, а обычный
    /// минимальный образ, и ответ «нечем» здесь такой же законный, как список узлов.
    pub fn trace_cmd_posix(host: &str, hops: u8) -> String {
        // `tracepath` идёт первым не по алфавиту: он затем и написан, чтобы работать без
        // прав root, а `traceroute` открывает сырой сокет и обычному пользователю почти
        // везде отказывает. Живой стенд это и показал: по SSH мы приходим не root.
        format!(
            "if command -v tracepath >/dev/null 2>&1; then echo TOOL=tracepath; \
               tracepath -n -m {hops} '{host}' 2>&1; \
             elif command -v traceroute >/dev/null 2>&1; then echo TOOL=traceroute; \
               traceroute -n -m {hops} -w 1 -q 1 '{host}' 2>&1; \
             else echo TOOL=none; fi"
        )
    }

    pub fn trace_cmd_windows(host: &str, hops: u8) -> String {
        format!(
            "powershell -NoProfile -NonInteractive -Command \"echo TOOL=tracert; \
             tracert -d -h {hops} -w 1000 '{host}'\""
        )
    }

    /// Разрешение имени на POSIX-системе.
    pub fn dns_cmd_posix(name: &str) -> String {
        format!(
            "if command -v getent >/dev/null 2>&1; then echo TOOL=getent; \
               getent ahosts '{name}' 2>/dev/null | awk '{{print \"A=\"$1}}' | sort -u; \
             elif command -v nslookup >/dev/null 2>&1; then echo TOOL=nslookup; \
               nslookup '{name}' 2>/dev/null | awk '/^Address: /{{print \"A=\"$2}}'; \
             else echo TOOL=none; fi"
        )
    }

    pub fn dns_cmd_windows(name: &str) -> String {
        format!(
            "powershell -NoProfile -NonInteractive -Command \"echo TOOL=powershell; \
             Resolve-DnsName -Name '{name}' -ErrorAction SilentlyContinue | \
             Where-Object {{ $_.IPAddress }} | ForEach-Object {{ echo \\\"A=$($_.IPAddress)\\\" }}\""
        )
    }

    /// Разбор ответа разрешения имени.
    pub fn parse_dns(name: &str, stdout: &str) -> Value {
        let tool = tag(stdout, "TOOL=").unwrap_or_default();
        if tool == "none" {
            return json!({
                "name": name,
                "from": "server",
                "error": "На сервере нечем разрешить имя: нет ни getent, ни nslookup",
            });
        }
        let mut addrs: Vec<String> = Vec::new();
        for line in stdout.lines() {
            if let Some(a) = line.trim().strip_prefix("A=") {
                let a = a.trim().to_string();
                // Один и тот же адрес приезжает по разу на каждый тип сокета.
                if !a.is_empty() && !addrs.contains(&a) {
                    addrs.push(a);
                }
            }
        }
        json!({ "name": name, "from": "server", "tool": tool, "addresses": addrs })
    }

    /// Разбор вывода трассировки.
    ///
    /// Разбирается не формат конкретной программы, а то общее, что есть у всех трёх:
    /// номер узла в начале строки, адрес где-то в ней и время в миллисекундах. У
    /// `traceroute`, `tracepath` и `tracert` порядок и обрамление разные — у последней
    /// адрес вообще в конце строки, — но эти три вещи есть у каждой. Заодно разбор не
    /// зависит от языка системы: слова мы не читаем.
    pub fn parse_trace(host: &str, stdout: &str) -> Value {
        let tool = tag(stdout, "TOOL=").unwrap_or_default();
        if tool == "none" {
            return json!({
                "host": host,
                "from_server": true,
                "error": "На сервере нечем построить маршрут: нет ни traceroute, ни tracepath",
            });
        }
        let mut hops: Vec<Value> = Vec::new();
        for line in stdout.lines() {
            let t = line.trim();
            let Some(num) = leading_number(t) else { continue };
            let addr = t.split_whitespace().find_map(clean_addr);
            let ms = first_ms(t);
            hops.push(json!({
                "n": num,
                // Узел мог не ответить — это законный исход, и звёздочки в выводе значат
                // именно его. Пустой адрес честнее выдуманного.
                "addr": addr,
                "ms": ms,
            }));
        }
        if hops.is_empty() {
            // Ни одного узла — это не «маршрут пустой», такого не бывает. Значит программа
            // не отработала, и сказать почему надо её же словами. Самый частый случай:
            // `traceroute` есть, но открыть сырой сокет обычному пользователю не дают.
            let сказано = stdout
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with("TOOL="))
                .unwrap_or("программа ничего не ответила");
            let низкий = сказано.to_lowercase();
            let подсказка =
                if низкий.contains("not permitted") || низкий.contains("permission denied") {
                    " — нужны права root либо установленный tracepath, он умеет без них"
                } else {
                    ""
                };
            return json!({
                "host": host,
                "from_server": true,
                "tool": tool,
                "error": format!("Маршрут построить не удалось: {сказано}{подсказка}"),
            });
        }
        json!({ "host": host, "from_server": true, "tool": tool, "hops": hops })
    }

    /// Номер узла в начале строки. `1:` у tracepath, `1` у остальных.
    fn leading_number(line: &str) -> Option<u32> {
        let first = line.split_whitespace().next()?;
        first.trim_end_matches(&[':', '?'][..]).parse().ok()
    }

    /// Адрес, если этот кусок строки на него похож.
    fn clean_addr(word: &str) -> Option<String> {
        let w = word.trim_matches(&['(', ')', ',', '[', ']'][..]);
        w.parse::<std::net::IpAddr>().ok().map(|ip| ip.to_string())
    }

    /// Первое время в миллисекундах: `0.402 ms`, `1ms` или `<1 ms`.
    fn first_ms(line: &str) -> Option<f64> {
        let words: Vec<&str> = line.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            if let Some(num) = w.strip_suffix("ms") {
                // `1ms` — число приклеено к единице.
                if let Some(v) = number(num) {
                    return Some(v);
                }
            }
            // `0.402 ms` — число и единица порознь.
            if *w == "ms" && i > 0 {
                if let Some(v) = number(words[i - 1]) {
                    return Some(v);
                }
            }
        }
        None
    }

    /// Число из куска строки. Учитывает две привычки живых программ: запятую вместо точки
    /// на русской локали и `<1` в значении «меньше миллисекунды» у `tracert`. Без второго
    /// у быстрых узлов время просто пропадало.
    fn number(word: &str) -> Option<f64> {
        word.trim().trim_start_matches('<').replace(',', ".").parse().ok()
    }

    /// Значение первой строки, начинающейся с метки.
    fn tag(stdout: &str, prefix: &str) -> Option<String> {
        stdout
            .lines()
            .find_map(|l| l.trim().strip_prefix(prefix))
            .map(|v| v.trim().to_string())
    }
}

#[cfg(test)]
mod remote_tests {
    use super::remote::*;

    #[test]
    fn адрес_с_посторонними_символами_не_уходит_в_команду() {
        // Адрес подставляется в командную строку на чужой машине. Отказ здесь дешевле
        // любого экранирования: список разрешённого проверяется взглядом.
        assert!(check_host("example.com").is_ok());
        assert!(check_host("192.168.0.1").is_ok());
        assert!(check_host("fe80::1").is_ok());
        assert!(check_host("a'; rm -rf / #").is_err());
        assert!(check_host("$(whoami)").is_err());
        assert!(check_host("`id`").is_err());
        assert!(check_host("a b").is_err());
        assert!(check_host("").is_err());
    }

    #[test]
    fn адрес_с_дефиса_прочтётся_как_ключ() {
        assert!(check_host("-z").is_err());
    }

    #[test]
    fn открытый_и_закрытый_порт_различаются() {
        let v = parse_port("db", 3306, "TOOL=nc
R=open
");
        assert_eq!(v["ok"], true);
        assert_eq!(v["tool"], "nc");
        assert_eq!(v["from"], "server");

        let v = parse_port("db", 3306, "TOOL=bash
R=closed
");
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("закрыт"));
    }

    #[test]
    fn несостоявшаяся_проверка_не_выдаётся_за_закрытый_порт() {
        // Это разные новости: «порт закрыт» и «мы не смогли проверить».
        let v = parse_port("db", 3306, "TOOL=none
");
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("нечем"));

        let v = parse_port("db", 3306, "");
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("не ответил"));
    }

    #[test]
    fn адреса_из_getent_не_повторяются() {
        // `getent ahosts` печатает один адрес по разу на каждый тип сокета.
        let out = "TOOL=getent
A=93.184.216.34
A=93.184.216.34
A=2606:2800:220::1
";
        let v = parse_dns("example.com", out);
        let a = v["addresses"].as_array().unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0], "93.184.216.34");
    }

    #[test]
    fn отсутствие_утилит_разрешения_имени_объясняется() {
        let v = parse_dns("example.com", "TOOL=none
");
        assert!(v["error"].as_str().unwrap().contains("нечем"));
        assert!(v.get("addresses").is_none());
    }

    #[test]
    fn маршрут_читается_у_всех_трёх_программ() {
        // Форматы разные, общее только три вещи: номер узла, адрес и время. У tracert
        // адрес вообще в конце строки, а у traceroute — в начале и в скобках.
        let tr = "TOOL=traceroute\n 1  172.23.0.1  0.004 ms\n 2  192.168.0.1  0.402 ms\n";
        let v = parse_trace("1.1.1.1", tr);
        let hops = v["hops"].as_array().unwrap();
        assert_eq!(hops.len(), 2);
        assert_eq!(hops[0]["n"], 1);
        assert_eq!(hops[0]["addr"], "172.23.0.1");
        assert_eq!(hops[1]["ms"], 0.402);

        let win = "TOOL=tracert\n  1     1 ms     1 ms     1 ms  192.168.0.1\n";
        let v = parse_trace("1.1.1.1", win);
        let h = &v["hops"][0];
        assert_eq!(h["n"], 1);
        // Адрес в конце строки — его всё равно надо найти.
        assert_eq!(h["addr"], "192.168.0.1");
        assert_eq!(h["ms"], 1.0);

        let tp = "TOOL=tracepath\n 1:  192.168.0.1  0.123ms\n";
        let v = parse_trace("1.1.1.1", tp);
        assert_eq!(v["hops"][0]["n"], 1);
        assert_eq!(v["hops"][0]["ms"], 0.123);
    }

    #[test]
    fn настоящий_вывод_tracert_разбирается_целиком() {
        // Снято с живой машины, не придумано. Здесь сразу три особенности: заголовок и
        // хвост без номера узла, `<1 ms` у быстрых узлов и молчащий узел со звёздочками.
        let out = concat!(
            "TOOL=tracert\n",
            "Tracing route to 1.1.1.1 over a maximum of 4 hops\n",
            "\n",
            "  1     1 ms    <1 ms    <1 ms  10.20.0.1 \n",
            "  2     *        *        *     Request timed out.\n",
            "  3     2 ms     1 ms     1 ms  93.100.100.1 \n",
            "  4     1 ms     1 ms     1 ms  93.100.0.132 \n",
            "\n",
            "Trace complete.\n"
        );
        let v = parse_trace("1.1.1.1", out);
        let hops = v["hops"].as_array().unwrap();
        // Ровно четыре: заголовок и «Trace complete» узлами не являются.
        assert_eq!(hops.len(), 4, "лишние или потерянные узлы: {v}");
        assert_eq!(hops[0]["addr"], "10.20.0.1");
        assert_eq!(hops[1]["n"], 2);
        assert!(hops[1]["addr"].is_null(), "у молчащего узла взялся адрес");
        assert_eq!(hops[3]["addr"], "93.100.0.132");
    }

    #[test]
    fn меньше_миллисекунды_это_число_а_не_пропуск() {
        // `<1 ms` — обычная запись Windows. Раньше время у таких узлов терялось целиком.
        let v = parse_trace("x", "TOOL=tracert\n  1    <1 ms    <1 ms    <1 ms  10.0.0.1\n");
        assert_eq!(v["hops"][0]["ms"], 1.0);
    }

    #[test]
    fn молчащий_узел_остаётся_без_адреса_а_не_пропадает() {
        // Звёздочки означают «узел не ответил». Пропустить такую строку значит сдвинуть
        // нумерацию и соврать о длине маршрута.
        let out = "TOOL=traceroute\n 1  10.0.0.1  0.5 ms\n 2  * * *\n 3  1.1.1.1  7.0 ms\n";
        let v = parse_trace("1.1.1.1", out);
        let hops = v["hops"].as_array().unwrap();
        assert_eq!(hops.len(), 3);
        assert!(hops[1]["addr"].is_null(), "у молчащего узла взялся адрес");
        assert!(hops[1]["ms"].is_null());
        assert_eq!(hops[2]["n"], 3);
    }

    #[test]
    fn заголовок_traceroute_не_считается_узлом() {
        // Первая строка вывода — «traceroute to 1.1.1.1 (1.1.1.1), 3 hops max…». Номера
        // узла в начале у неё нет, и попасть в список она не должна.
        let out = "TOOL=traceroute\ntraceroute to 1.1.1.1 (1.1.1.1), 3 hops max, 46 byte packets\n 1  10.0.0.1  0.5 ms\n";
        let v = parse_trace("1.1.1.1", out);
        assert_eq!(v["hops"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn отказ_в_правах_не_выдаётся_за_пустой_маршрут() {
        // Живой стенд поймал ровно это: `traceroute` на месте, но обычному пользователю
        // не дают открыть сырой сокет. Раньше отсюда получался пустой список узлов —
        // то есть «маршрута нет», хотя на деле его просто не построили.
        let out = "TOOL=traceroute\ntraceroute: socket(AF_INET,3,1): Operation not permitted\n";
        let v = parse_trace("1.1.1.1", out);
        assert!(v.get("hops").is_none(), "взялся маршрут: {v}");
        let err = v["error"].as_str().unwrap();
        assert!(err.contains("Operation not permitted"), "потеряли слова программы: {err}");
        assert!(err.contains("tracepath"), "нет подсказки, чем это лечится: {err}");
    }

    #[test]
    fn непривилегированная_программа_идёт_первой() {
        // Порядок важен: `tracepath` затем и написан, чтобы работать без прав root.
        let c = trace_cmd_posix("1.1.1.1", 5);
        let tp = c.find("tracepath").expect("нет tracepath");
        let tr = c.find("traceroute").expect("нет traceroute");
        assert!(tp < tr, "traceroute проверяется раньше tracepath");
    }

    #[test]
    fn отсутствие_traceroute_объясняется_словами() {
        // На голом Debian нет ни traceroute, ни tracepath, ни ping. Это обычный образ,
        // а не редкость, и пустой список тут читался бы как «маршрута нет».
        let v = parse_trace("1.1.1.1", "TOOL=none\n");
        assert!(v.get("hops").is_none());
        assert!(v["error"].as_str().unwrap().contains("нечем"));
    }

    #[test]
    fn скан_со_своей_машины_находит_живой_сокет() {
        // Разбор ответов сервера проверяется отдельно, а здесь — сам обход диапазона:
        // пачки, таймаут, сборка списка. Сокет поднимаем сами, чтобы тест не зависел от
        // того, что случайно слушает на машине.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = l.local_addr().unwrap().port();
            // Диапазон вокруг него: соседние порты почти наверняка свободны, и если тест
            // однажды поймает чужой — это будет видно по лишнему числу в списке.
            let from = port.saturating_sub(2).max(1);
            let to = port.saturating_add(2);
            let v = super::port_scan("127.0.0.1".into(), from, to, Some(300))
                .await
                .expect("скан");
            let open: Vec<u64> = v["open"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_u64().unwrap())
                .collect();
            assert!(open.contains(&(port as u64)), "не нашли свой же сокет: {v}");
            assert_eq!(v["scanned"], (to as u32 - from as u32 + 1));
        });
    }

    #[test]
    fn диапазон_проверяется_до_начала_работы() {
        use super::parse_range;
        assert!(parse_range(1, 1024).is_ok());
        assert!(parse_range(22, 22).is_ok());
        // Начало больше конца — это не пустой диапазон, а опечатка.
        assert!(parse_range(100, 10).is_err());
        assert!(parse_range(0, 10).is_err());
        // Тысячи портов — задача для nmap, а не для окна утилит.
        assert!(parse_range(1, 5000).is_err());
        let err = parse_range(1, 5000).unwrap_err();
        assert!(err.contains("5000"), "в отказе нет размера диапазона: {err}");
    }

    #[test]
    fn открытые_порты_собираются_и_сортируются() {
        let out = "TOOL=nc
P=80
P=22
P=22
P=443
";
        let v = parse_scan("srv", 1, 1024, out);
        let open: Vec<u64> = v["open"].as_array().unwrap().iter().map(|p| p.as_u64().unwrap()).collect();
        // По возрастанию и без повторов: список читают глазами.
        assert_eq!(open, vec![22, 80, 443]);
        assert_eq!(v["scanned"], 1024);
    }

    #[test]
    fn порт_вне_запрошенного_диапазона_отбрасывается() {
        // Если в ответе оказалось что-то за пределами запроса, значит мы читаем не то,
        // и молча подмешивать это в список нельзя.
        let v = parse_scan("srv", 20, 25, "TOOL=nc
P=22
P=8080
");
        let open = v["open"].as_array().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0], 22);
    }

    #[test]
    fn скан_без_утилит_объясняется_а_не_выдаёт_пустой_список() {
        // Пустой список означал бы «всё закрыто» — это другое утверждение.
        let v = parse_scan("srv", 1, 10, "TOOL=none
");
        assert!(v.get("open").is_none());
        assert!(v["error"].as_str().unwrap().contains("нечем"));
    }

    #[test]
    fn команда_проверки_порта_умеет_обе_ветки() {
        // На голом Debian нет `nc`, на Alpine нет `bash` — ни одна ветка по отдельности
        // не покрывает даже наш стенд.
        let c = port_cmd_posix("example.com", 443, 3);
        assert!(c.contains("nc -z"));
        assert!(c.contains("/dev/tcp/example.com/443"));
        assert!(c.contains("TOOL=none"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_parsing() {
        assert_eq!(parse_host_port("example.com", 443).unwrap(), ("example.com".into(), 443));
        assert_eq!(parse_host_port("h:8080", 443).unwrap(), ("h".into(), 8080));
        assert_eq!(parse_host_port("[::1]:8443", 443).unwrap(), ("::1".into(), 8443));
    }

    #[test]
    fn subnet_slash24() {
        let v = subnet_calc("192.168.1.0/24").unwrap();
        assert_eq!(v["network"], "192.168.1.0");
        assert_eq!(v["hostCount"], 254);
        assert_eq!(v["broadcast"], "192.168.1.255");
    }

    #[test]
    fn hash_sha256_known() {
        let v = hash_text("sha256", "abc").unwrap();
        assert_eq!(
            v["hex"].as_str().unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn jwt_decode_sample() {
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.sig";
        let v = jwt_decode(token).unwrap();
        assert_eq!(v["payload"]["sub"], "1234567890");
    }
}
