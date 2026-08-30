//! Общая обвязка интеграционных тестов: параметры стенда и подключение к нему.
//!
//! Стенд поднимается снаружи — `scripts/ssh-stand/up.sh`. Здесь только чтение переменных
//! и удобные конструкторы конфигов сервера в том же виде, в каком их отдаёт фронтенд.

#![allow(dead_code)] // каждый тестовый файл берёт отсюда своё подмножество

use serde_json::{json, Value};

pub struct Stand {
    pub host: String,
    pub debian_port: u16,
    pub alpine_port: u16,
    pub user: String,
    pub password: String,
    pub key_path: String,
    /// Как Alpine видно ИЗНУТРИ сети стенда — нужно для jump-цепочки: дальний хост
    /// открывается каналом с промежуточного, и проброшенный на хост порт оттуда не виден.
    pub alpine_internal: String,
    /// Второй порт того же Debian — только для теста, который портит запомненный
    /// отпечаток. Отдельный порт даёт отдельную запись в `known_hosts`, поэтому тест
    /// не мешает соседям, идущим параллельно на основной порт.
    pub hostkey_port: u16,
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("не задана {name} — подними стенд через scripts/ssh-stand/up.sh и возьми переменные оттуда")
    })
}

impl Stand {
    /// Параметры стенда из окружения.
    ///
    /// Отсутствие любой переменной — падение с внятным текстом, а не тихий пропуск:
    /// зелёный тест, который ничего не проверил, хуже красного.
    pub fn from_env() -> Self {
        assert!(
            std::env::var_os("SEREIN_CONFIG_DIR").is_some(),
            "не задана SEREIN_CONFIG_DIR: тесты пишут в профиль (подтверждённые ключи хостов) и не должны трогать профиль живого пользователя"
        );
        Self {
            host: env("SEREIN_STAND_HOST"),
            debian_port: env("SEREIN_STAND_DEBIAN_PORT").parse().expect("порт Debian"),
            alpine_port: env("SEREIN_STAND_ALPINE_PORT").parse().expect("порт Alpine"),
            user: env("SEREIN_STAND_USER"),
            password: env("SEREIN_STAND_PASSWORD"),
            key_path: env("SEREIN_STAND_KEY"),
            alpine_internal: env("SEREIN_STAND_ALPINE_INTERNAL"),
            hostkey_port: env("SEREIN_STAND_HOSTKEY_PORT").parse().expect("порт для теста ключа"),
        }
    }

    pub fn by_password(&self, port: u16) -> Value {
        json!({
            "host": self.host,
            "port": port,
            "username": self.user,
            "authType": "password",
            "password": self.password,
            "connectTimeout": 20,
        })
    }

    pub fn by_key(&self, port: u16) -> Value {
        json!({
            "host": self.host,
            "port": port,
            "username": self.user,
            "authType": "key",
            "privateKeyPath": self.key_path,
            "connectTimeout": 20,
        })
    }

    pub fn home(&self) -> String {
        format!("/home/{}", self.user)
    }
}

pub fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("рантайм")
}

/// Замок для тестов, которые правят `known_hosts.json` напрямую.
///
/// Ирония: приложение свою гонку на этом файле уже не имеет (в `knownhosts` есть замок),
/// а вот тесты, которые лезут в файл мимо приложения, переоткрывают её сами. Кто читает
/// файл целиком или переписывает его — берёт этот замок.
pub fn profile_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}
