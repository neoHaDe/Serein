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

mod common;
use common::{profile_lock, rt, Stand};
use serde_json::{json, Value};

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

    let _guard = profile_lock().lock().unwrap_or_else(|e| e.into_inner());
    let file = serein_lib::store::config_dir().join("known_hosts.json");
    let text = std::fs::read_to_string(&file).expect("файл отпечатков");
    for port in [s.debian_port, s.alpine_port] {
        let id = format!("{}:{}", s.host, port);
        assert!(text.contains(&id), "потерян отпечаток {id} в {text}");
    }
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn changed_host_key_is_refused_when_there_is_nobody_to_ask() {
    // Смена ключа хоста — это либо переустановленный сервер, либо чужой сервер на том же
    // адресе. Спросить пользователя можно только в живой сессии; когда спрашивать некого
    // (восстановление туннеля, массовый прогон), молча доверять нельзя ни при каких
    // обстоятельствах. Проверяем именно этот путь.
    //
    // Ключ хоста не подменяем перезапуском контейнера — записываем заведомо чужой
    // отпечаток в профиль. Для проверяемой логики это одно и то же, а тест не зависит
    // от того, есть ли у него доступ к docker.
    let s = Stand::from_env();
    let host_id = format!("{}:{}", s.host, s.hostkey_port);

    // Правим ровно свою запись и ничего не восстанавливаем обратно. Первый вариант этого
    // теста читал файл целиком, а в конце writeback'ом возвращал снимок — и стирал
    // отпечатки, которые за это время записали соседние тесты. Ровно та же гонка, которую
    // мы только что чинили в приложении, воспроизведённая в тесте.
    //
    // Восстанавливать нечего: порт 2203 существует только ради этой проверки, и чужой
    // отпечаток на нём никому не мешает.
    {
        let _guard = profile_lock().lock().unwrap_or_else(|e| e.into_inner());
        let file = serein_lib::store::config_dir().join("known_hosts.json");
        let mut data: serde_json::Map<String, Value> = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        data.insert(
            host_id.clone(),
            json!("SHA256:0000000000000000000000000000000000000000000"),
        );
        std::fs::write(&file, serde_json::to_string_pretty(&data).unwrap())
            .expect("подмена отпечатка");
    }

    let res = rt().block_on(serein_lib::ssh::connect_client(vec![s.by_key(s.hostkey_port)]));
    assert!(res.is_err(), "сервер со сменившимся ключом принимать нельзя");
}

