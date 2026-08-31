//! Типизированные ошибки бэкенда.
//!
//! Внутри Rust держим варианты с контекстом; наружу в Tauri-команды уходит строка через
//! `Display`. Так проще тестировать и не терять фазу сбоя при конвертации.

use std::fmt;

/// На каком шаге SSH-сессии произошла ошибка — для диагностики в UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Connect,
    HostKey,
    Auth,
    Jump,
    Shell,
}

impl SessionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::HostKey => "hostkey",
            Self::Auth => "auth",
            Self::Jump => "jump",
            Self::Shell => "shell",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SereinError {
    AuthRejected {
        host: String,
        auth_type: String,
    },
    ConnectTimeout {
        host: String,
        secs: u64,
    },
    ConnectFailed {
        host: String,
        detail: String,
        phase: SessionPhase,
    },
    ProxyJump {
        host: String,
        detail: String,
    },
    HostKey {
        detail: String,
    },
    Cancelled,
    Timeout {
        op: String,
    },
    Config(String),
    Crypto(String),
    Protocol(String),
    EmptyChain,
    Other(String),
}

pub type Result<T> = std::result::Result<T, SereinError>;

impl SereinError {
    pub fn phase(&self) -> Option<SessionPhase> {
        match self {
            Self::ConnectFailed { phase, .. } => Some(*phase),
            Self::AuthRejected { .. } => Some(SessionPhase::Auth),
            Self::ConnectTimeout { .. } => Some(SessionPhase::Connect),
            Self::ProxyJump { .. } => Some(SessionPhase::Jump),
            Self::HostKey { .. } => Some(SessionPhase::HostKey),
            _ => None,
        }
    }
}

impl fmt::Display for SereinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthRejected { host, auth_type } => {
                let msg = match auth_type.as_str() {
                    "key" => format!(
                        "Сервер {host} отклонил ключ — проверьте путь, passphrase и что ключ добавлен в authorized_keys"
                    ),
                    "agent" => format!("Сервер {host} отклонил ключ из SSH-агента"),
                    _ => format!("Сервер {host} отклонил пароль или второй фактор"),
                };
                write!(f, "{msg}")
            }
            Self::ConnectTimeout { host, secs } => {
                write!(f, "Подключение к {host} не удалось за {secs} с")
            }
            Self::ConnectFailed { host, detail, .. } => {
                write!(f, "Подключение к {host} не удалось: {detail}")
            }
            Self::ProxyJump { host, detail } => {
                write!(f, "ProxyJump к {host} не удался: {detail}")
            }
            Self::HostKey { detail } => write!(f, "{detail}"),
            Self::Cancelled => write!(f, "Операция отменена"),
            Self::Timeout { op } => write!(f, "Превышено время ожидания: {op}"),
            Self::Config(s) | Self::Crypto(s) | Self::Protocol(s) | Self::Other(s) => write!(f, "{s}"),
            Self::EmptyChain => write!(f, "Пустая цепочка хостов"),
        }
    }
}

impl From<SereinError> for String {
    fn from(e: SereinError) -> String {
        e.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_rejected_carries_phase() {
        let e = SereinError::AuthRejected {
            host: "srv".into(),
            auth_type: "key".into(),
        };
        assert_eq!(e.phase(), Some(SessionPhase::Auth));
        assert!(e.to_string().contains("srv"));
        assert!(e.to_string().contains("ключ"));
    }

    #[test]
    fn connect_timeout_message() {
        let e = SereinError::ConnectTimeout {
            host: "h".into(),
            secs: 15,
        };
        assert!(e.to_string().contains("15"));
    }
}

/// Ошибка команды открытия сессии — с фазой, а не одной строкой.
///
/// Фаза нужна фронтенду, чтобы решить, повторять ли попытку. Повторять сетевой сбой
/// осмысленно, а неверный пароль — нет: он правильным не станет, зато пять попыток подряд
/// дают шесть неудачных аутентификаций за полминуты — порог типичного `fail2ban` и
/// блокировки доменной учётной записи.
#[derive(Debug, serde::Serialize)]
pub struct OpenError {
    pub message: String,
    pub phase: Option<&'static str>,
}

impl From<SereinError> for OpenError {
    fn from(e: SereinError) -> Self {
        Self {
            message: e.to_string(),
            phase: e.phase().map(SessionPhase::as_str),
        }
    }
}

impl From<String> for OpenError {
    fn from(message: String) -> Self {
        Self { message, phase: None }
    }
}

impl From<&str> for OpenError {
    fn from(message: &str) -> Self {
        Self { message: message.to_string(), phase: None }
    }
}
