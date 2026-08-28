//! Podklyuchenie k lokalnomu SSH-agentu (OpenSSH Authentication Agent / ssh-agent).

use byteorder::{BigEndian, ByteOrder};
use russh_keys::agent::client::AgentClient;
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const WIN_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";
const MAX_AGENT_REPLY: usize = 256 * 1024;

pub fn agent_unavailable_hint() -> &'static str {
    "SSH-agent nedostupen. Na Windows: sluzhba OpenSSH Authentication Agent + ssh-add. Na Linux/macOS: eval \"$(ssh-agent)\" i ssh-add."
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
        return Err("Nekorrektnoe soobshchenie SSH-agenta".into());
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
        return Err("Slishkom bolshoy otvet SSH-agenta".into());
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.map_err(|e| agent_err(e))?;
    let mut out = Vec::with_capacity(4 + len);
    out.extend_from_slice(&len_buf);
    out.extend(body);
    Ok(out)
}

pub async fn authenticate_with_agent(
    handle: &mut russh::client::Handle<crate::ssh::ClientHandler>,
    user: &str,
) -> Result<bool, String> {
    let mut agent = connect_agent().await?;
    let keys = agent.request_identities().await.map_err(|e| agent_err(e))?;
    if keys.is_empty() {
        return Err("V SSH-agente net klyuchey. Dobavte klyuch: ssh-add.".into());
    }
    for key in keys {
        let (agent_back, ok) = handle.authenticate_future(user, key, agent).await;
        agent = agent_back;
        match ok {
            Ok(true) => return Ok(true),
            Ok(false) => continue,
            Err(e) => return Err(format!("Oshibka SSH-agenta: {e}")),
        }
    }
    Ok(false)
}