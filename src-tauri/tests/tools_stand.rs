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

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn скан_диапазона_находит_ровно_открытый_порт() {
    // Узкий диапазон вокруг порта базы: 3306 обязан найтись, соседние — нет. Проверяем на
    // обеих машинах, потому что ветки команд у них разные, и ошибка в одной из них
    // выглядела бы как «на этом сервере ничего не открыто».
    let s = Stand::from_env();
    rt().block_on(async {
        for (порт, ветка) in [(s.debian_port, "bash"), (s.alpine_port, "nc")] {
            let cmd = remote::scan_cmd_posix(&s.mariadb_host, 3304, 3307, 1);
            let out = run(&s, порт, &cmd).await;
            let v = remote::parse_scan(&s.mariadb_host, 3304, 3307, &out);
            assert_eq!(v["tool"], ветка, "не та ветка команд, ответ:\n{out}");
            let open: Vec<u64> = v["open"]
                .as_array()
                .expect("нет списка портов")
                .iter()
                .map(|p| p.as_u64().unwrap())
                .collect();
            assert_eq!(open, vec![3306], "ответ стенда:\n{out}");
        }
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn alpine_не_выдаёт_отказ_в_правах_за_пустой_маршрут() {
    // Этот тест написан по следам живой проверки. Вручную `traceroute` на Alpine работал —
    // потому что `docker exec` заходит под root. По SSH мы приходим обычным пользователем,
    // и сырой сокет ему открыть не дают. Разбор отдавал на это пустой список узлов, то
    // есть «маршрута нет» вместо «маршрут не построили» — ровно та ложь, которой здесь
    // быть не должно.
    let s = Stand::from_env();
    rt().block_on(async {
        let cmd = remote::trace_cmd_posix(&s.mariadb_host, 5);
        let out = run(&s, s.alpine_port, &cmd).await;
        let v = remote::parse_trace(&s.mariadb_host, &out);

        if v.get("hops").is_some() {
            // Если однажды на образе появится tracepath или traceroute станет setuid —
            // маршрут построится, и это тоже верный исход. Тогда он обязан быть непустым.
            let hops = v["hops"].as_array().unwrap();
            assert!(!hops.is_empty(), "пустой список узлов выдан за маршрут: {v}");
            assert!(hops[0]["addr"].is_string(), "у первого узла нет адреса: {v}");
        } else {
            let err = v["error"].as_str().unwrap_or("");
            assert!(!err.is_empty(), "отказ без объяснения: {v}");
            assert!(
                err.contains("not permitted") || err.contains("нечем"),
                "непонятная причина отказа: {err}"
            );
        }
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn debian_честно_говорит_что_маршрут_строить_нечем() {
    // На этом образе нет ни traceroute, ни tracepath, ни ping. Это обычный минимальный
    // Debian, а не редкость, и пустой список узлов читался бы как «маршрута нет».
    let s = Stand::from_env();
    rt().block_on(async {
        let cmd = remote::trace_cmd_posix(&s.mariadb_host, 5);
        let out = run(&s, s.debian_port, &cmd).await;
        let v = remote::parse_trace(&s.mariadb_host, &out);
        assert!(v.get("hops").is_none(), "взялся маршрут там, где нечем: {v}");
        assert!(
            v["error"].as_str().unwrap_or("").contains("нечем"),
            "отказ без объяснения: {v}"
        );
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn alpine_делает_http_запрос_через_wget() {
    // Веб-служба стенда наружу не опубликована: с машины, где идут тесты, её адреса не
    // существует вовсе. Это и есть тот вопрос, ради которого утилита умеет «с сервера».
    // На Alpine нет `curl`, только busybox-`wget` — значит работает вторая ветка.
    let s = Stand::from_env();
    rt().block_on(async {
        let url = format!("http://{}:80/", s.web_host);
        let cmd = remote::http_cmd_posix(&url, "GET", 5);
        let out = run(&s, s.alpine_port, &cmd).await;
        let v = remote::parse_http(&url, &out);
        assert_eq!(v["tool"], "wget", "ожидали ветку wget, ответ:\n{out}");
        assert_eq!(v["status"], 200, "ответ стенда:\n{out}");
        let names: Vec<String> = v["headers"]
            .as_array()
            .expect("нет заголовков")
            .iter()
            .map(|h| h["name"].as_str().unwrap_or("").to_lowercase())
            .collect();
        assert!(names.contains(&"content-type".to_string()), "нет Content-Type: {v}");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn debian_честно_говорит_что_запрос_делать_нечем() {
    // На минимальном Debian нет ни `curl`, ни `wget`. Это обычный образ, а не редкость,
    // и молчаливый пустой ответ читался бы как «служба не отвечает».
    let s = Stand::from_env();
    rt().block_on(async {
        let url = format!("http://{}:80/", s.web_host);
        let cmd = remote::http_cmd_posix(&url, "GET", 5);
        let out = run(&s, s.debian_port, &cmd).await;
        let v = remote::parse_http(&url, &out);
        assert!(v.get("status").is_none(), "взялся код ответа там, где нечем: {v}");
        assert!(
            v["error"].as_str().unwrap_or("").contains("нечем"),
            "отказ без объяснения: {v}"
        );
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn закрытая_дверь_не_выдаётся_за_ответ_службы() {
    // Порт есть, но HTTP там не живёт. Кода ответа не будет, и вместо него должно быть
    // сказано словами самой программы — иначе непонятно, служба легла или адрес не тот.
    let s = Stand::from_env();
    rt().block_on(async {
        let url = format!("http://{}:9/", s.web_host);
        let cmd = remote::http_cmd_posix(&url, "GET", 3);
        let out = run(&s, s.alpine_port, &cmd).await;
        let v = remote::parse_http(&url, &out);
        assert!(v.get("status").is_none(), "выдумали код ответа: {v}");
        assert!(!v["error"].as_str().unwrap_or("").is_empty(), "отказ без причины: {v}");
    });
}
