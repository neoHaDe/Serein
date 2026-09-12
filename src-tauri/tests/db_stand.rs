//! Базы данных через SSH-канал — против настоящих PostgreSQL, MySQL, MariaDB и Redis.
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
    db::open(id.clone(), "сессия-стенда", &h, p).await.expect("подключение к базе");
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

/// Один и тот же набор проверок для MariaDB и для MySQL 8.
///
/// Отдельные машины они не ради разнообразия: MariaDB проверяет пароль плагином
/// `mysql_native_password`, MySQL 8 — `caching_sha2_password` с обменом открытым ключом.
/// Это две разные ветки нашего кода входа, и общий у них только SQL.
async fn проверить_mysql(s: &Stand, host: &str) {
    let id = open(s, params(Kind::Mysql, host, "probe", Some("probe"))).await;

    let out = db::query(&id, "SELECT 1 AS число, 'привет' AS текст")
        .await
        .expect("запрос");
    let cols: Vec<&str> = out["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(cols, vec!["число", "текст"], "колонки пришли не те");
    let rows = out["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["число"], "1");
    // Юникод и в именах колонок, и в значениях: между нами SSH-канал и своя упаковка.
    assert_eq!(rows[0]["текст"], "привет");

    // NULL обязан отличаться от пустой строки — иначе по таблице нельзя судить о данных.
    let out = db::query(&id, "SELECT NULL AS пусто, '' AS строка").await.expect("запрос");
    assert!(out["rows"][0]["пусто"].is_null(), "NULL приехал не как NULL");
    assert_eq!(out["rows"][0]["строка"], "");

    // Запрос без выборки сообщает про изменённые строки, а не про пустую таблицу.
    db::query(&id, "CREATE TEMPORARY TABLE проба (id INT)").await.expect("создание таблицы");
    let out = db::query(&id, "INSERT INTO проба VALUES (1), (2), (3)")
        .await
        .expect("вставка");
    assert_eq!(out["affected"], 3, "не посчитались изменённые строки");

    // Ошибку базы показываем её словами, а не своими.
    let err = db::query(&id, "SELECT * FROM таблицы_нет").await.unwrap_err();
    assert!(
        err.to_lowercase().contains("таблицы_нет") || err.contains("1146"),
        "ошибка без подробностей: {err}"
    );

    db::close(&id);
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn mariadb_отвечает_через_ssh_канал() {
    let s = Stand::from_env();
    rt().block_on(async {
        let host = s.mariadb_host.clone();
        проверить_mysql(&s, &host).await;
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn mysql8_проходит_вход_с_обменом_ключом() {
    // Самая рискованная ветка: `caching_sha2_password` при первом входе требует полной
    // аутентификации. Пароль открытым текстом мы не шлём — просим у сервера открытый
    // ключ и шифруем. Проверить это можно только на живом MySQL 8.
    let s = Stand::from_env();
    rt().block_on(async {
        let host = s.mysql_host.clone();
        проверить_mysql(&s, &host).await;
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn длинный_ответ_mysql_не_рвётся_на_границе_пакета() {
    // Тело длиной ровно 0xFFFFFF протокол продолжает следующим пакетом. Склейка — наш
    // код, и ошибка в ней проявляется только на больших ответах.
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, params(Kind::Mysql, &s.mariadb_host, "probe", Some("probe"))).await;
        let out = db::query(&id, "SELECT REPEAT('я', 400000) AS длинное")
            .await
            .expect("запрос");
        let v = out["rows"][0]["длинное"].as_str().expect("значение");
        // Склейка проверяется не длиной показанного, а тем, что мы знаем полную длину:
        // значение обрезается для показа (иначе таблица получила бы мегабайты в одной
        // ячейке), но в пометке стоит настоящий размер - 400000 символов по два байта.
        assert!(v.contains("обрезано"), "длинное значение обязано быть помечено: {v:.80}");
        assert!(
            v.contains(&(400000 * 2).to_string()),
            "в пометке должен стоять полный размер значения: {:.120}",
            &v[v.len().saturating_sub(120)..]
        );
        assert_eq!(out["truncated"], true, "об обрезке надо сказать и на уровне ответа");

        // И главное: соединение после большого ответа осталось исправным. Разъехавшаяся
        // склейка проявилась бы именно здесь - следующий запрос прочёл бы хвост прошлого.
        let after = db::query(&id, "SELECT 7 AS сверка").await.expect("запрос после большого ответа");
        assert_eq!(after["rows"][0]["сверка"], "7");
        db::close(&id);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn неверный_пароль_mysql_отвергается_с_текстом() {
    let s = Stand::from_env();
    rt().block_on(async {
        let h = ssh::connect_client(vec![s.by_key(s.debian_port)])
            .await
            .expect("подключение к серверу");
        let mut p = params(Kind::Mysql, &s.mariadb_host, "probe", Some("probe"));
        p.password = Some("не тот пароль".into());
        let err = db::open("test-bad-mysql".into(), "сессия-стенда", &h, p).await.unwrap_err();
        // Пустая строка вместо причины оставила бы человека гадать.
        assert!(!err.trim().is_empty(), "отказ без объяснения");
        assert!(
            err.to_lowercase().contains("probe") || err.contains("1045") || err.contains("Access"),
            "непонятный отказ: {err}"
        );
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn хранимая_процедура_не_разъезжает_соединение() {
    // Процедура отвечает несколькими результатами подряд. Показываем первый, но остаток
    // обязаны вычитать: иначе следующий запрос прочтёт хвост предыдущего. Проявляется это
    // не ошибкой, а неверными данными — поэтому проверяем именно вторым запросом.
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, params(Kind::Mysql, &s.mariadb_host, "probe", Some("probe"))).await;

        db::query(&id, "DROP PROCEDURE IF EXISTS проба_двух").await.expect("уборка");
        db::query(&id, "CREATE PROCEDURE проба_двух() BEGIN SELECT 1 AS первый; END")
            .await
            .expect("создание процедуры");

        let out = db::query(&id, "CALL проба_двух()").await.expect("вызов");
        assert_eq!(out["rows"][0]["первый"], "1");

        // Вот здесь и вылезал бы хвост: следующий запрос обязан вернуть своё.
        let out = db::query(&id, "SELECT 42 AS после").await.expect("запрос после вызова");
        assert_eq!(out["columns"][0], "после", "колонки приехали от прошлого запроса");
        assert_eq!(out["rows"][0]["после"], "42");

        db::query(&id, "DROP PROCEDURE проба_двух").await.expect("уборка");
        db::close(&id);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn закрытие_базы_не_роняет_процесс() {
    // Это не абстрактная проверка: закрытие панели убивало приложение целиком. `russh`
    // в деструкторе канала вызывает `tokio::spawn`, чтобы попрощаться с сервером, а вне
    // рантайма такой вызов паникует — и паника в деструкторе не разворачивается, процесс
    // просто исчезает. Если тест упадёт с abort, значит закрытие снова идёт мимо рантайма.
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, params(Kind::Mysql, &s.mariadb_host, "probe", Some("probe"))).await;
        db::close(&id);
        // Даём задаче прощания отработать: она уходит в рантайм, а не выполняется здесь.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // Соединения больше нет, и запрос обязан сказать об этом, а не ждать вечно.
        assert!(db::query(&id, "SELECT 1").await.is_err());
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn базы_закрываются_вместе_со_своей_сессией() {
    // Канал живёт внутри SSH-сессии. Если сессия ушла, а запись осталась, следующий
    // запрос уходил бы в мёртвый канал и ждал ответа, которого не будет.
    let s = Stand::from_env();
    rt().block_on(async {
        let h = ssh::connect_client(vec![s.by_key(s.debian_port)])
            .await
            .expect("подключение к серверу");
        let id = format!("test-{}", uuid::Uuid::new_v4());
        db::open(
            id.clone(),
            "сессия-которую-закроют",
            &h,
            params(Kind::Mysql, &s.mariadb_host, "probe", Some("probe")),
        )
        .await
        .expect("подключение к базе");

        assert_eq!(db::count_for_session("сессия-которую-закроют"), 1);
        db::close_session("сессия-которую-закроют");
        assert_eq!(db::count_for_session("сессия-которую-закроют"), 0);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(db::query(&id, "SELECT 1").await.is_err());
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn открытую_базу_можно_найти_по_сессии() {
    // Так откреплённое окно узнаёт про соединение: своей памяти у него нет — это
    // отдельный веб-контекст, — а база открыта и живёт в приложении. Без этого вопроса
    // отделение панели выглядело бы обрывом связи, хотя рвать было нечего.
    let s = Stand::from_env();
    rt().block_on(async {
        let h = ssh::connect_client(vec![s.by_key(s.debian_port)])
            .await
            .expect("подключение к серверу");
        let id = format!("test-{}", uuid::Uuid::new_v4());
        db::open(
            id.clone(),
            "сессия-с-окном",
            &h,
            params(Kind::Mysql, &s.mariadb_host, "probe", Some("probe")),
        )
        .await
        .expect("подключение к базе");

        let found = db::for_session("сессия-с-окном").expect("база не нашлась по сессии");
        assert_eq!(found["id"], id.as_str());
        assert_eq!(found["kind"], "mysql");
        assert_eq!(found["port"], 3306);

        // О чужой сессии выдумывать нельзя: там своё окно и, может быть, своя база.
        assert!(db::for_session("сессия-без-базы").is_none());

        db::close_session("сессия-с-окном");
        assert!(db::for_session("сессия-с-окном").is_none());
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn несколько_выборок_в_одном_запросе_не_смешиваются() {
    // Здесь раньше был не просто неверный показ, а падение: колонки запоминались от первой
    // строки и применялись ко второй выборке, у которой их меньше, а закреплённая
    // библиотека на обращении к отсутствующей колонке паникует.
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, params(Kind::Postgres, &s.pg_host, "probe", Some("probe"))).await;
        let out = db::query(&id, "SELECT 1 AS a, 2 AS b; SELECT 3 AS c")
            .await
            .expect("запрос из двух выборок");

        let sets = out["sets"].as_array().expect("наборы");
        assert_eq!(sets.len(), 2, "две выборки - два набора: {out}");
        let first: Vec<&str> = sets[0]["columns"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
        let second: Vec<&str> = sets[1]["columns"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
        assert_eq!(first, vec!["a", "b"]);
        assert_eq!(second, vec!["c"], "у второй выборки свои колонки");
        assert_eq!(sets[1]["rows"][0]["c"], "3");

        // Одинаковые имена колонок не съедают друг друга: строка - словарь, и второе
        // значение затирало первое.
        let dup = db::query(&id, "SELECT 1 AS a, 2 AS a").await.expect("одинаковые имена");
        let cols: Vec<&str> = dup["columns"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
        assert_eq!(cols, vec!["a", "a#2"]);
        assert_eq!(dup["rows"][0]["a"], "1");
        assert_eq!(dup["rows"][0]["a#2"], "2");

        db::close(&id);
    });
}
