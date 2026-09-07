//! Утилиты, выполняемые **на сервере** — против настоящих Debian и Alpine.
//!
//! Проверять это на записанных ответах бессмысленно: весь смысл ветвления в том, что
//! набор утилит на живых машинах разный. На голом Debian нет `nc`, зато есть `bash`
//! с его `/dev/tcp`; на Alpine ровно наоборот. Каждая ветка покрывает ровно одну из
//! машин стенда, и без обеих не работает ни одна.
//!
//! Отдельная ценность здесь в адресах: `mariadb` — имя внутри сети стенда. С машины,
//! на которой идут тесты, оно не разрешается и порт его недоступен. Это и есть тот
//! случай, ради которого затевалась проверка «с сервера»: вопрос «вижу ли я» и вопрос
//! «видит ли сервер» имеют разные ответы.
//!
//! Запуск: `cargo test --test tools_stand -- --ignored --test-threads=1`

mod common;
use common::{rt, Stand};

use serein_lib::ssh;
use serein_lib::tools::remote;

async fn run(s: &Stand, port: u16, cmd: &str) -> String {
    let h = ssh::connect_client(vec![s.by_key(port)])
        .await
        .expect("подключение к серверу");
    let (_, out, _) = ssh::exec(&h, cmd, None).await.expect("команда");
    out
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn debian_проверяет_порт_через_bash() {
    // На этом образе нет `nc`, и работать обязана вторая ветка — `/dev/tcp`.
    let s = Stand::from_env();
    rt().block_on(async {
        let cmd = remote::port_cmd_posix(&s.mariadb_host, 3306, 3);
        let out = run(&s, s.debian_port, &cmd).await;
        let v = remote::parse_port(&s.mariadb_host, 3306, &out);
        assert_eq!(v["ok"], true, "ответ стенда:\n{out}");
        assert_eq!(v["tool"], "bash", "ожидали ветку bash, ответ:\n{out}");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn alpine_проверяет_порт_через_nc() {
    // А на этом нет `bash` — значит должна отработать первая ветка.
    let s = Stand::from_env();
    rt().block_on(async {
        let cmd = remote::port_cmd_posix(&s.mariadb_host, 3306, 3);
        let out = run(&s, s.alpine_port, &cmd).await;
        let v = remote::parse_port(&s.mariadb_host, 3306, &out);
        assert_eq!(v["ok"], true, "ответ стенда:\n{out}");
        assert_eq!(v["tool"], "nc", "ожидали ветку nc, ответ:\n{out}");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn закрытый_порт_виден_как_закрытый() {
    // Порт 9 (discard) на базе не слушает никто. Если проверка скажет «открыт», значит
    // мы принимаем за успех что-то другое — например, код возврата не той команды.
    let s = Stand::from_env();
    rt().block_on(async {
        for port in [s.debian_port, s.alpine_port] {
            let cmd = remote::port_cmd_posix(&s.mariadb_host, 9, 3);
            let out = run(&s, port, &cmd).await;
            let v = remote::parse_port(&s.mariadb_host, 9, &out);
            assert_eq!(v["ok"], false, "порт 9 выдан за открытый, ответ:\n{out}");
            // И это должен быть внятный отказ, а не «не смогли проверить».
            assert!(
                v["error"].as_str().unwrap_or("").contains("закрыт"),
                "неверная причина отказа: {v}"
            );
        }
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn сервер_разрешает_имя_которого_снаружи_не_существует() {
    // `mariadb` живёт только в сети стенда. Ровно та задача, ради которой всё затевалось:
    // со своей машины такое имя не разрешается, а с сервера — да.
    let s = Stand::from_env();
    rt().block_on(async {
        for port in [s.debian_port, s.alpine_port] {
            let cmd = remote::dns_cmd_posix(&s.mariadb_host);
            let out = run(&s, port, &cmd).await;
            let v = remote::parse_dns(&s.mariadb_host, &out);
            let addrs = v["addresses"].as_array().expect("нет списка адресов");
            assert!(!addrs.is_empty(), "имя не разрешилось, ответ:\n{out}");
            // Адреса не должны повторяться: getent печатает по строке на тип сокета.
            let mut uniq: Vec<&str> = addrs.iter().filter_map(|a| a.as_str()).collect();
            let было = uniq.len();
            uniq.sort_unstable();
            uniq.dedup();
            assert_eq!(было, uniq.len(), "адреса приехали с повторами: {v}");
        }
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn несуществующее_имя_не_выдаёт_пустой_список_за_успех() {
    // Пустой список адресов — это «не разрешилось», и выглядеть он должен именно так,
    // а не как удачная проверка без результата.
    let s = Stand::from_env();
    rt().block_on(async {
        let name = "такого-имени-нет.invalid";
        let cmd = remote::dns_cmd_posix(name);
        let out = run(&s, s.debian_port, &cmd).await;
        let v = remote::parse_dns(name, &out);
        assert!(v["addresses"].as_array().unwrap().is_empty(), "ответ:\n{out}");
    });
}
