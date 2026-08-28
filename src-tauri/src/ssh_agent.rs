//! Подключение к локальному SSH-агенту (OpenSSH Authentication Agent / ssh-agent).

use base64::Engine;
use byteorder::{BigEndian, ByteOrder};
use russh_keys::agent::client::AgentClient;
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const WIN_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";
const MAX_AGENT_REPLY: usize = 256 * 1024;

pub fn agent_unavailable_hint() -> &'static str {
    "SSH-агент недоступен. На Windows: служба «OpenSSH Authentication Agent» + ssh-add. На Linux/macOS: eval \"$(ssh-agent)\" и ssh-add."
}

fn agent_err(e: impl std::fmt::Display) -> String {
    format!("{} ({e})", agent_unavailable_hint())
}

pub async fn connect_agent_stream(
) -> Result<impl AsyncRead + AsyncWrite + Unpin, String> {
    #[cfg(unix)]
    {
        let path = std::env::var("SSH_AUTH_SOCK").map_err(|_| agent_unavailable_hint().to_string())?;
        tokio::net::UnixStream::connect(path)
            .await
            .map_err(|e| agent_err(e))
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        if let Ok(var) = std::env::var("SSH_AUTH_SOCK") {
            if let Ok(stream) = ClientOptions::new().open(&var) {
                return Ok(stream);
            }
        }
        ClientOptions::new()
            .open(WIN_PIPE)
            .map_err(|e| agent_err(e))
    }
}

pub async fn connect_agent(
) -> Result<AgentClient<impl AsyncRead + AsyncWrite + Unpin>, String> {
    Ok(AgentClient::connect(connect_agent_stream().await?))
}

pub async fn agent_roundtrip(payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() < 4 {
        return Err("Некорректное сообщение SSH-агента".into());
    }
    let mut stream = connect_agent_stream().await?;
    stream.write_all(payload).await.map_err(|e| agent_err(e))?;
    stream.flush().await.map_err(|e| agent_err(e))?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => agent_unavailable_hint().to_string(),
        _ => agent_err(e),
    })?;
    let len = BigEndian::read_u32(&len_buf) as usize;
    if len > MAX_AGENT_REPLY {
        return Err("Слишком большой ответ SSH-агента".into());
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.map_err(|e| agent_err(e))?;
    let mut out = Vec::with_capacity(4 + len);
    out.extend_from_slice(&len_buf);
    out.extend(body);
    Ok(out)
}

// ---------- Список ключей агента ----------
//
// `AgentClient::request_identities` из russh-keys выбрасывает комментарий ключа
// (`let _comment = r.read_string()?`), а в UI выбирать удобнее именно по комментарию
// («hade@pc»), а не по отпечатку. Поэтому список читаем сами по протоколу агента:
// запрос SSH_AGENTC_REQUEST_IDENTITIES, ответ SSH_AGENT_IDENTITIES_ANSWER.

const MSG_REQUEST_IDENTITIES: u8 = 11;
const MSG_IDENTITIES_ANSWER: u8 = 12;

/// Читает `string` протокола SSH: 4 байта длины + тело. Курсор двигается за телом.
fn take_string<'a>(buf: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let len = take_u32(buf, pos)? as usize;
    let end = pos.checked_add(len)?;
    let out = buf.get(*pos..end)?;
    *pos = end;
    Some(out)
}

fn take_u32(buf: &[u8], pos: &mut usize) -> Option<u32> {
    let end = pos.checked_add(4)?;
    let n = BigEndian::read_u32(buf.get(*pos..end)?);
    *pos = end;
    Some(n)
}

/// Отпечаток в формате OpenSSH из сырого блоба публичного ключа.
fn fingerprint_of(blob: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(blob);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    )
}

/// Разбирает тело ответа агента (без 4 байт длины) в список ключей.
pub fn parse_identities(body: &[u8]) -> Result<Vec<AgentIdentity>, String> {
    match body.first() {
        Some(&MSG_IDENTITIES_ANSWER) => {}
        Some(_) => return Err("SSH-агент ответил отказом на запрос списка ключей".into()),
        None => return Err("Пустой ответ SSH-агента".into()),
    }
    let mut pos = 1usize;
    let count = take_u32(body, &mut pos).ok_or("Обрезанный ответ SSH-агента")?;
    let mut out = Vec::new();
    for _ in 0..count {
        let blob = take_string(body, &mut pos).ok_or("Обрезанный ключ в ответе SSH-агента")?;
        let comment = take_string(body, &mut pos).ok_or("Обрезанный комментарий в ответе SSH-агента")?;
        // Первая строка внутри блоба — имя алгоритма («ssh-ed25519», «ssh-rsa», …).
        let mut bpos = 0usize;
        let algo = take_string(blob, &mut bpos)
            .map(|a| String::from_utf8_lossy(a).to_string())
            .unwrap_or_else(|| "неизвестный".into());
        out.push(AgentIdentity {
            algo,
            comment: String::from_utf8_lossy(comment).to_string(),
            fingerprint: fingerprint_of(blob),
        });
    }
    Ok(out)
}

pub struct AgentIdentity {
    pub algo: String,
    pub comment: String,
    pub fingerprint: String,
}

impl AgentIdentity {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "algo": self.algo,
            "comment": self.comment,
            "fingerprint": self.fingerprint,
        })
    }
}

/// Ключи, загруженные в локальный агент. Ошибка — если агента нет или он недоступен.
pub async fn list_identities() -> Result<Vec<AgentIdentity>, String> {
    let req = [0, 0, 0, 1, MSG_REQUEST_IDENTITIES];
    let resp = agent_roundtrip(&req).await?;
    let body = resp.get(4..).ok_or("Пустой ответ SSH-агента")?;
    parse_identities(body)
}

pub async fn authenticate_with_agent(
    handle: &mut russh::client::Handle<crate::ssh::ClientHandler>,
    user: &str,
    preferred: Option<&str>,
) -> Result<bool, String> {
    let mut agent = connect_agent().await?;
    let keys = agent.request_identities().await.map_err(|e| agent_err(e))?;
    if keys.is_empty() {
        return Err("В SSH-агенте нет ключей. Добавьте ключ: ssh-add.".into());
    }

    // Профиль может указывать конкретный ключ по отпечатку — тогда не перебираем все
    // подряд: лишние попытки на сервере с `MaxAuthTries 2` приводят к отказу ещё до
    // нужного ключа.
    let keys: Vec<_> = match preferred.filter(|p| !p.is_empty()) {
        Some(want) => {
            let picked: Vec<_> = keys
                .into_iter()
                .filter(|k| format!("SHA256:{}", k.fingerprint()) == want)
                .collect();
            if picked.is_empty() {
                return Err(format!(
                    "Выбранного ключа ({want}) нет в SSH-агенте. Добавьте его (ssh-add) или выберите другой в настройках сервера."
                ));
            }
            picked
        }
        None => keys,
    };

    for key in keys {
        let (agent_back, ok) = handle.authenticate_future(user, key, agent).await;
        agent = agent_back;
        match ok {
            Ok(true) => return Ok(true),
            Ok(false) => continue,
            Err(e) => return Err(format!("Ошибка SSH-агента: {e}")),
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::{fingerprint_of, parse_identities, MSG_IDENTITIES_ANSWER};

    fn ssh_string(bytes: &[u8]) -> Vec<u8> {
        let mut out = (bytes.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(bytes);
        out
    }

    fn answer(keys: &[(&str, &str)]) -> Vec<u8> {
        let mut body = vec![MSG_IDENTITIES_ANSWER];
        body.extend_from_slice(&(keys.len() as u32).to_be_bytes());
        for (algo, comment) in keys {
            let mut blob = ssh_string(algo.as_bytes());
            blob.extend(ssh_string(b"key-material"));
            body.extend(ssh_string(&blob));
            body.extend(ssh_string(comment.as_bytes()));
        }
        body
    }

    #[test]
    fn parses_two_keys_with_comments() {
        let body = answer(&[("ssh-ed25519", "hade@pc"), ("ssh-rsa", "backup key")]);
        let keys = parse_identities(&body).expect("разбор");
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].algo, "ssh-ed25519");
        assert_eq!(keys[0].comment, "hade@pc");
        assert_eq!(keys[1].algo, "ssh-rsa");
        assert_eq!(keys[1].comment, "backup key");
        assert!(keys[0].fingerprint.starts_with("SHA256:"));
        assert_ne!(keys[0].fingerprint, keys[1].fingerprint);
    }

    #[test]
    fn empty_agent_is_not_an_error() {
        let keys = parse_identities(&answer(&[])).expect("разбор");
        assert!(keys.is_empty());
    }

    #[test]
    fn refusal_and_garbage_are_rejected() {
        assert!(parse_identities(&[]).is_err());
        assert!(parse_identities(&[5]).is_err()); // SSH_AGENT_FAILURE
        // Заявлено 2 ключа, тело содержит один — не паникуем, а сообщаем об обрезке.
        let mut truncated = answer(&[("ssh-ed25519", "hade@pc")]);
        truncated[1..5].copy_from_slice(&2u32.to_be_bytes());
        assert!(parse_identities(&truncated).is_err());
    }

    #[test]
    fn fingerprint_is_openssh_shaped() {
        let fp = fingerprint_of(b"blob");
        assert!(fp.starts_with("SHA256:"));
        assert!(!fp.ends_with('=')); // base64 без паддинга, как в OpenSSH
    }
}