//! Базы данных через SSH-канал — против настоящих PostgreSQL и Redis.
//!
//! В стенде их порты наружу не опубликованы вовсе: базы видны только изнутри сети, как и
//! на нормально настроенном сервере. Поэтому единственный способ до них дойти — канал
//! `direct-tcpip` внутри уже установленной SSH-сессии, и проверяется именно он.
//!
//! Запуск: `cargo test --test db_stand -- --ignored --test-threads=1`

mod common;
use common::{rt, Stand};

use serein_lib::db::{self, Kind, Params};
use serein_lib::ssh;

fn params(kind: Kind, host: &str, user: &str, db: Option<&str>) -> Params {
    Params {
        kind,
        host: Some(host.to_string()),
        port: None,
        user: Some(user.to_string()),
        password: Some("probe-pass".to_string()),
        database: db.map(str::to_string),
    }
}

/// Подключается к серверу стенда и открывает через него базу.
async fn open(s: &Stand, p: Params) -> String {
    let h = ssh::connect_client(vec![s.by_key(s.debian_port)])
        .await
        .expect("подключение к серверу");
    let id = format!("test-{}", uuid::Uuid::new_v4());
    db::open(id.clone(), &h, p).await.expect("подключение к базе");
    id
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn postgres_отвечает_через_ssh_канал() {
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, params(Kind::Postgres, &s.pg_host, "probe", Some("probe"))).await;
        let out = db::query(&id, "SELECT 1 AS число, 'привет' AS текст")
            .await
            .expect("запрос");

        let cols: Vec<&str> = out["columns"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
        assert_eq!(cols, vec!["число", "текст"], "колонки пришли не те");

        let rows = out["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["число"], "1");
        // Юникод в значениях обязан доехать целым: между нами SSH-канал и своя упаковка.
        assert_eq!(rows[0]["текст"], "привет");

        db::close(&id);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn postgres_различает_null_и_пустую_строку() {
    // В таблице это разные вещи, и путать их нельзя: пустая строка — значение, NULL — его
    // отсутствие. При наивной сборке ответа оба превращаются в пустую ячейку.
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, params(Kind::Postgres, &s.pg_host, "probe", Some("probe"))).await;
        let out = db::query(&id, "SELECT NULL::text AS пусто, '' AS строка")
            .await
            .expect("запрос");
        let row = &out["rows"].as_array().unwrap()[0];
        assert!(row["пусто"].is_null(), "NULL превратился в {:?}", row["пусто"]);
        assert_eq!(row["строка"], "");
        db::close(&id);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn postgres_возвращает_число_изменённых_строк() {
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, params(Kind::Postgres, &s.pg_host, "probe", Some("probe"))).await;
        db::query(&id, "CREATE TEMP TABLE проба (имя text)").await.expect("создание");
        let out = db::query(&id, "INSERT INTO проба VALUES ('раз'), ('два')")
            .await
            .expect("вставка");
        assert_eq!(out["affected"], 2, "не посчитаны вставленные строки");
        db::close(&id);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn ошибка_запроса_приходит_с_текстом_от_базы() {
    // Без разбора ошибки наружу уходит «db error» без единой подробности, и человек не
    // понимает, что именно он написал не так.
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, params(Kind::Postgres, &s.pg_host, "probe", Some("probe"))).await;
        let err = db::query(&id, "SELECT * FROM таблицы_которой_нет")
            .await
            .expect_err("ожидали ошибку");
        assert!(
            err.contains("42P01") || err.to_lowercase().contains("relation"),
            "ошибка без подробностей: {err}"
        );
        db::close(&id);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn redis_читает_и_пишет_через_ssh_канал() {
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, params(Kind::Redis, &s.redis_host, "", None)).await;

        let out = db::query(&id, "PING").await.expect("PING");
        assert_eq!(out["rows"][0]["значение"], "PONG");

        // Значение с пробелами — тот случай, ради которого команда режется с оглядкой на
        // кавычки: иначе на сервер уедет только первое слово.
        db::query(&id, r#"SET проба "два слова""#).await.expect("SET");
        let got = db::query(&id, "GET проба").await.expect("GET");
        assert_eq!(got["rows"][0]["значение"], "два слова");

        db::query(&id, "DEL проба").await.expect("DEL");
        db::close(&id);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn redis_разворачивает_список_в_строки() {
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, params(Kind::Redis, &s.redis_host, "", None)).await;
        db::query(&id, "DEL список").await.ok();
        db::query(&id, "RPUSH список раз два три").await.expect("RPUSH");

        let out = db::query(&id, "LRANGE список 0 -1").await.expect("LRANGE");
        let rows = out["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 3, "список не развёрнут в строки: {rows:?}");
        assert_eq!(rows[0]["значение"], "раз");
        assert_eq!(rows[2]["значение"], "три");

        db::query(&id, "DEL список").await.ok();
        db::close(&id);
    });
}

/// Начало следующего сервиса в compose: перевод строки и два пробела отступа.
const SERVICE_SEP: &str = "\n  ";

#[test]
fn базы_в_стенде_не_публикуют_порты_наружу() {
    // Проверка постановки задачи, а не поведения: базы должны быть доступны только изнутри
    // сети стенда. Стучаться при этом в 127.0.0.1 бессмысленно — там может слушать
    // собственный Postgres разработчика, и тест «докажет» ровно ничего. Поэтому смотрим
    // саму конфигурацию: если у сервиса появится публикация порта, стенд перестанет
    // проверять путь через SSH, и заметить это надо сразу.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../scripts/ssh-stand/docker-compose.yml");
    let text = std::fs::read_to_string(path).expect("файл стенда");

    for name in ["postgres:", "redis:"] {
        let from = text.find(name).unwrap_or_else(|| panic!("сервис {name} пропал из стенда"));
        // Читаем до начала следующего сервиса — им считается строка с двумя пробелами.
        let rest = &text[from..];
        let till = rest[1..].find(SERVICE_SEP).map(|k| k + 1).unwrap_or(rest.len());
        let block = &rest[..till];
        assert!(
            !block.contains("ports:"),
            "сервис {name} публикует порты наружу — тесты перестали проверять путь через SSH"
        );
    }
}
