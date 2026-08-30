//! Матрица авторизации против настоящих SSH-серверов.
//!
//! Стенд поднимается отдельно (`scripts/ssh-stand/up.sh`) и живёт в Docker: Debian с
//! обычным OpenSSH и Alpine с BusyBox. Второй здесь не для галочки — на таких машинах
//! (роутеры, встраиваемые системы) ломается то, что на Debian работает молча.
//!
//! Тесты помечены `#[ignore]` намеренно. Обычный `cargo test` должен оставаться быстрым и
//! ни от чего не зависеть; эти же требуют поднятого стенда, и лучше честно видеть их
//! пропущенными, чем прятать проверку за «если переменных нет — тихо считаем успехом».
//! Запуск: `cargo test -- --ignored` с переменными из `up.sh`.
//!
//! ⚠ `SEREIN_CONFIG_DIR` обязателен. Тесты подтверждают ключи хостов, а это запись в
//! профиль — гадить в профиль живого пользователя они не имеют права.

use serde_json::{json, Value};

/// Параметры стенда. Отсутствие любого — повод упасть с внятным текстом, а не молча
/// «пройти»: зелёный тест, который ничего не проверил, хуже красного.
struct Stand {
    host: String,
    debian_port: u16,
    alpine_port: u16,
    user: String,
    password: String,
    key_path: String,
    /// Как Alpine видно ИЗНУТРИ сети стенда. Для jump-цепочки нужен именно этот адрес:
    /// дальний хост открывается каналом с промежуточного, и проброшенный на хост порт
    /// оттуда не виден вовсе.
    alpine_internal: String,
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("не задана {name} — подними стенд через scripts/ssh-stand/up.sh и возьми переменные оттуда")
    })
}

impl Stand {
    fn from_env() -> Self {
        assert!(
            std::env::var_os("SEREIN_CONFIG_DIR").is_some(),
            "не задана SEREIN_CONFIG_DIR: тесты подтверждают ключи хостов и писали бы в профиль живого пользователя"
        );
        Self {
            host: env("SEREIN_STAND_HOST"),
            debian_port: env("SEREIN_STAND_DEBIAN_PORT").parse().expect("порт Debian"),
            alpine_port: env("SEREIN_STAND_ALPINE_PORT").parse().expect("порт Alpine"),
            user: env("SEREIN_STAND_USER"),
            password: env("SEREIN_STAND_PASSWORD"),
            key_path: env("SEREIN_STAND_KEY"),
            alpine_internal: env("SEREIN_STAND_ALPINE_INTERNAL"),
        }
    }

    fn by_password(&self, port: u16) -> Value {
        json!({
            "host": self.host,
            "port": port,
            "username": self.user,
            "authType": "password",
            "password": self.password,
            "connectTimeout": 20,
        })
    }

    fn by_key(&self, port: u16) -> Value {
        json!({
            "host": self.host,
            "port": port,
            "username": self.user,
            "authType": "key",
            "privateKeyPath": self.key_path,
            "connectTimeout": 20,
        })
    }
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("рантайм")
}

/// Подключиться и выполнить команду, вернув (код, stdout).
fn run(server: Value, command: &str) -> (i32, String) {
    rt().block_on(async move {
        let handle = serein_lib::ssh::connect_client(vec![server])
            .await
            .expect("подключение к стенду");
        let (code, out, err) = serein_lib::ssh::exec(&handle, command, None)
            .await
            .expect("выполнение команды");
        assert!(err.is_empty() || code != 0, "неожиданный stderr: {err}");
        (code, out)
    })
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn password_auth_works_on_debian() {
    let s = Stand::from_env();
    let (code, out) = run(s.by_password(s.debian_port), "id -un");
    assert_eq!(code, 0);
    assert_eq!(out.trim(), s.user);
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn key_auth_works_on_debian() {
    let s = Stand::from_env();
    let (code, out) = run(s.by_key(s.debian_port), "id -un");
    assert_eq!(code, 0);
    assert_eq!(out.trim(), s.user);
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn password_auth_works_on_busybox() {
    // Alpine здесь не дубль Debian: у него BusyBox-шелл и урезанные утилиты, и часть
    // наших команд (мониторинг, список процессов) на нём ведёт себя иначе.
    let s = Stand::from_env();
    let (code, out) = run(s.by_password(s.alpine_port), "id -un");
    assert_eq!(code, 0);
    assert_eq!(out.trim(), s.user);
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn key_auth_works_on_busybox() {
    let s = Stand::from_env();
    let (code, out) = run(s.by_key(s.alpine_port), "id -un");
    assert_eq!(code, 0);
    assert_eq!(out.trim(), s.user);
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn wrong_password_is_refused_without_hanging() {
    let s = Stand::from_env();
    let mut bad = s.by_password(s.debian_port);
    bad["password"] = json!("это-точно-не-пароль");
    let started = std::time::Instant::now();
    let res = rt().block_on(serein_lib::ssh::connect_client(vec![bad]));
    assert!(res.is_err(), "с неверным паролем пускать нельзя");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "отказ должен приходить быстро, а не по таймауту"
    );
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn nonzero_exit_code_reaches_the_caller() {
    // Код возврата — это половина смысла массового прогона: без него «выполнилось» и
    // «выполнилось успешно» неотличимы.
    let s = Stand::from_env();
    let (code, _) = run(s.by_password(s.debian_port), "exit 42");
    assert_eq!(code, 42);
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn utf8_output_survives_the_wire() {
    // Вывод режется на пачки по границе UTF-8; кириллица и эмодзи ловят ошибку, при
    // которой символ разрывается между пачками и превращается в мусор.
    let s = Stand::from_env();
    let (code, out) = run(
        s.by_password(s.debian_port),
        "printf 'привет-мир-\\xF0\\x9F\\x94\\x91\\n'",
    );
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "привет-мир-🔑");
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn jump_chain_reaches_the_far_host() {
    // Цепочка: заходим на Alpine через Debian. Проверяем не «подключилось», а что команда
    // выполнилась именно на дальнем хосте — иначе jump молча превращается в прямое
    // подключение и никто этого не замечает.
    let s = Stand::from_env();
    let out = rt().block_on(async {
        // Порядок важен и неочевиден: нулевой элемент — цель, последний — внешний
        // jump-хост, к которому подключаются первым. Перепутать местами — значит
        // молча проверить прямое подключение вместо цепочки.
        let target = json!({
            "host": s.alpine_internal,
            "port": 22,
            "username": s.user,
            "authType": "key",
            "privateKeyPath": s.key_path,
            "connectTimeout": 20,
        });
        let chain = vec![target, s.by_key(s.debian_port)];
        let handle = serein_lib::ssh::connect_client(chain)
            .await
            .expect("подключение через jump");
        let (_, out, _) = serein_lib::ssh::exec(&handle, "cat /etc/os-release", None)
            .await
            .expect("команда на дальнем хосте");
        out
    });
    assert!(out.contains("Alpine"), "команда ушла не на дальний хост: {out}");
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn parallel_first_contact_remembers_every_host() {
    // Регрессия, найденная этими же тестами: `remember` читал файл, добавлял запись и
    // писал обратно без замка. Два одновременных подключения к новым хостам читали один
    // снимок, и запись первого терялась. Массовый прогон ходит в четыре потока, туннели
    // восстанавливаются пачкой — то есть случай не выдуманный.
    let s = Stand::from_env();
    let debian = s.by_key(s.debian_port);
    let alpine = s.by_key(s.alpine_port);
    rt().block_on(async {
        let a = serein_lib::ssh::connect_client(vec![debian]);
        let b = serein_lib::ssh::connect_client(vec![alpine]);
        let (ra, rb) = tokio::join!(a, b);
        ra.expect("Debian");
        rb.expect("Alpine");
    });

    let file = serein_lib::store::config_dir().join("known_hosts.json");
    let text = std::fs::read_to_string(&file).expect("файл отпечатков");
    for port in [s.debian_port, s.alpine_port] {
        let id = format!("{}:{}", s.host, port);
        assert!(text.contains(&id), "потерян отпечаток {id} в {text}");
    }
}
