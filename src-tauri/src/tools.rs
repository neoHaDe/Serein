//! Локальные утилиты: порт, DNS, TLS, подсеть, хеши, JWT (P2.1).

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
