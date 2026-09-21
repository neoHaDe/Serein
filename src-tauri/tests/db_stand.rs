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

/// Показанный набор ответа: строки лежат в `sets`, верхние поля их не повторяют.
fn shown(v: &serde_json::Value) -> &serde_json::Value {
    &v["sets"][v["shown"].as_u64().unwrap_or(0) as usize]
}

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

        let rows = shown(&out)["rows"].as_array().unwrap();
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
        let row = &shown(&out)["rows"].as_array().unwrap()[0];
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
        assert_eq!(shown(&out)["rows"][0]["значение"], "PONG");

        // Значение с пробелами — тот случай, ради которого команда режется с оглядкой на
        // кавычки: иначе на сервер уедет только первое слово.
        db::query(&id, r#"SET проба "два слова""#).await.expect("SET");
        let got = db::query(&id, "GET проба").await.expect("GET");
        assert_eq!(shown(&got)["rows"][0]["значение"], "два слова");

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
        let rows = shown(&out)["rows"].as_array().unwrap();
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
// Этой пометки раньше не было, и тест не запускался нигде: в CI джоба rust гоняет только
// `--lib`, а стенд запускает лишь помеченные `ignore`.
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
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
    let rows = shown(&out)["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["число"], "1");
    // Юникод и в именах колонок, и в значениях: между нами SSH-канал и своя упаковка.
    assert_eq!(rows[0]["текст"], "привет");

    // NULL обязан отличаться от пустой строки — иначе по таблице нельзя судить о данных.
    let out = db::query(&id, "SELECT NULL AS пусто, '' AS строка").await.expect("запрос");
    assert!(shown(&out)["rows"][0]["пусто"].is_null(), "NULL приехал не как NULL");
    assert_eq!(shown(&out)["rows"][0]["строка"], "");

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
        let v = shown(&out)["rows"][0]["длинное"].as_str().expect("значение");
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
        assert_eq!(shown(&after)["rows"][0]["сверка"], "7");
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
        assert_eq!(shown(&out)["rows"][0]["первый"], "1");

        // Вот здесь и вылезал бы хвост: следующий запрос обязан вернуть своё.
        let out = db::query(&id, "SELECT 42 AS после").await.expect("запрос после вызова");
        assert_eq!(out["columns"][0], "после", "колонки приехали от прошлого запроса");
        assert_eq!(shown(&out)["rows"][0]["после"], "42");

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
        assert_eq!(shown(&dup)["rows"][0]["a"], "1");
        assert_eq!(shown(&dup)["rows"][0]["a#2"], "2");

        db::close(&id);
    });
}


fn mssql_params(s: &Stand) -> Params {
    // Пароль свой: SQL Server не стартует с простым, поэтому общий пароль стенда ему не годится.
    Params {
        kind: Kind::Mssql,
        host: Some(s.mssql_host.clone()),
        port: None,
        user: Some("sa".to_string()),
        password: Some("Probe-pass-1".to_string()),
        database: None,
    }
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn sql_server_отвечает_через_ssh_канал() {
    // Порт SQL Server наружу не опубликован: дойти до него можно только каналом внутри
    // SSH-сессии - и с шифрованием, которое новые установки требуют по умолчанию.
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, mssql_params(&s)).await;
        let out = db::query(
            &id,
            "SELECT 1 AS число, N'привет' AS текст, CAST(NULL AS int) AS пусто, '' AS пустая_строка",
        )
        .await
        .expect("запрос");
        let row = &shown(&out)["rows"][0];
        assert_eq!(row["число"], "1");
        assert_eq!(row["текст"], "привет", "юникод обязан доехать целым");
        assert!(row["пусто"].is_null(), "NULL остаётся NULL");
        assert_eq!(row["пустая_строка"], "", "пустая строка - не NULL");

        // Номер ошибки в тексте: по нему ошибку ищут, текст бывает переведён.
        let err = db::query(&id, "SELECT * FROM нет_такой_таблицы").await.expect_err("ошибка");
        assert!(err.contains("208"), "ожидался номер ошибки 208: {err}");
        db::close(&id);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn sql_server_отдаёт_наборы_даты_и_числа() {
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, mssql_params(&s)).await;
        let out = db::query(
            &id,
            "SELECT 1 AS a; SELECT CAST('2026-09-13T10:20:30' AS datetime2) AS когда, CAST(12.50 AS decimal(5,2)) AS сумма",
        )
        .await
        .expect("запрос из двух выборок");
        let sets = out["sets"].as_array().expect("наборы");
        assert_eq!(sets.len(), 2, "две выборки - два набора: {out}");
        assert_eq!(
            sets[1]["rows"][0]["когда"], "2026-09-13 10:20:30",
            "дата - датой, а не внутренним видом TDS"
        );
        assert!(
            sets[1]["rows"][0]["сумма"].as_str().unwrap_or("").starts_with("12.5"),
            "десятичное число текстом: {}",
            sets[1]["rows"][0]["сумма"]
        );

        // Соединение исправно после нескольких наборов: поток дочитан до конца.
        let again = db::query(&id, "SELECT 7 AS сверка").await.expect("повторный запрос");
        assert_eq!(shown(&again)["rows"][0]["сверка"], "7");
        db::close(&id);
    });
}


#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn sqlite_читается_через_sqlite3_на_сервере() {
    // У SQLite нет сетевого протокола - это файл. Запрос идёт через `sqlite3` на самом
    // сервере, SQL уходит на стандартный вход, а колонки обязаны прийти в своём порядке.
    let s = Stand::from_env();
    rt().block_on(async {
        let h = ssh::connect_client(vec![s.by_key(s.debian_port)])
            .await
            .expect("подключение к серверу");
        let (code, _, err) = ssh::exec(
            &h,
            "rm -f /tmp/serein-stand.db && sqlite3 /tmp/serein-stand.db \"CREATE TABLE t(b TEXT, a INTEGER); INSERT INTO t VALUES ('привет', 1), ('', NULL);\"",
            None,
        )
        .await
        .expect("создание базы");
        assert_eq!(code, 0, "база не создалась: {err}");

        let params = |db: &str| Params {
            kind: Kind::Sqlite,
            host: None,
            port: None,
            user: None,
            password: None,
            database: Some(db.to_string()),
        };
        let id = format!("test-{}", uuid::Uuid::new_v4());
        db::open(id.clone(), "сессия-стенда", &h, params("/tmp/serein-stand.db"))
            .await
            .expect("открытие базы");

        let out = db::query(&id, "SELECT b, a FROM t ORDER BY a IS NULL, a; SELECT count(*) AS n FROM t")
            .await
            .expect("запрос");
        let sets = out["sets"].as_array().expect("наборы");
        assert_eq!(sets.len(), 2, "две выборки - два набора: {out}");
        let cols: Vec<&str> = sets[0]["columns"]
            .as_array()
            .expect("колонки")
            .iter()
            .map(|c| c.as_str().expect("имя колонки"))
            .collect();
        assert_eq!(cols, vec!["b", "a"], "колонки в порядке запроса, а не по алфавиту");
        assert_eq!(sets[0]["rows"][0]["b"], "привет", "юникод обязан доехать целым");
        assert_eq!(sets[0]["rows"][0]["a"], "1");
        assert_eq!(sets[0]["rows"][1]["b"], "", "пустая строка - не NULL");
        assert!(sets[0]["rows"][1]["a"].is_null(), "NULL остаётся NULL");
        assert_eq!(sets[1]["rows"][0]["n"], "2");

        let err = db::query(&id, "SELECT * FROM нет_такой").await.expect_err("ошибка");
        assert!(err.contains("no such table"), "текст ошибки от sqlite3: {err}");
        db::close(&id);

        // Опечатка в пути не создаёт пустую базу.
        let lost = db::open(
            format!("test-{}", uuid::Uuid::new_v4()),
            "сессия-стенда",
            &h,
            params("/tmp/нет-такой-базы.db"),
        )
        .await
        .expect_err("файла нет");
        assert!(lost.contains("нет"), "{lost}");
        let (_, exists, _) = ssh::exec(&h, "test -e /tmp/нет-такой-базы.db && echo есть || echo нет", None)
            .await
            .expect("проверка файла");
        assert_eq!(exists.trim(), "нет", "пустой файл не должен появиться");
    });
}


fn mongo_params(s: &Stand, user: &str, password: &str) -> Params {
    Params {
        kind: Kind::Mongo,
        host: Some(s.mongo_host.clone()),
        port: None,
        user: Some(user.to_string()),
        password: Some(password.to_string()),
        database: Some("probe".to_string()),
    }
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn mongodb_отвечает_через_ssh_канал() {
    // Порт MongoDB наружу не опубликован, а вход по паролю включён: проверяется и канал
    // внутри SSH-сессии, и свой вход SCRAM.
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, mongo_params(&s, "probe", "probe-pass")).await;
        db::query(&id, "db.stand_items.deleteMany({})").await.expect("очистка");
        let put = db::query(
            &id,
            "db.stand_items.insertMany([{ b: 'привет', a: 1, вложенный: { x: [1, 2] } }, { b: '', a: null }])",
        )
        .await
        .expect("вставка");
        assert_eq!(put["affected"], 2, "{put}");

        let out = db::query(&id, "db.stand_items.find({}, { _id: 0 }).sort({ a: -1 })")
            .await
            .expect("выборка");
        let cols: Vec<&str> = out["columns"]
            .as_array()
            .expect("колонки")
            .iter()
            .map(|c| c.as_str().expect("имя колонки"))
            .collect();
        assert_eq!(cols, vec!["b", "a", "вложенный"], "поля в порядке документа, а не по алфавиту");
        let rows = shown(&out)["rows"].as_array().expect("строки");
        assert_eq!(rows[0]["b"], "привет", "юникод обязан доехать целым");
        assert_eq!(rows[0]["a"], "1");
        assert_eq!(rows[0]["вложенный"], "{ x: [ 1, 2 ] }");
        assert_eq!(rows[1]["b"], "", "пустая строка - не null");
        assert!(rows[1]["a"].is_null(), "null остаётся null");

        let n = db::query(&id, "db.stand_items.countDocuments({ a: { $gte: 1 } })")
            .await
            .expect("подсчёт");
        assert_eq!(shown(&n)["rows"][0]["количество"], "1");

        db::query(
            &id,
            "db.stand_items.insertOne({ _id: ObjectId('650000000000000000000001'), когда: ISODate('2026-09-13T10:20:30Z') })",
        )
        .await
        .expect("вставка с датой");
        let one = db::query(&id, "db.stand_items.findOne({ _id: ObjectId('650000000000000000000001') })")
            .await
            .expect("поиск по _id");
        assert_eq!(shown(&one)["rows"][0]["_id"], "ObjectId('650000000000000000000001')");
        assert!(
            shown(&one)["rows"][0]["когда"].as_str().unwrap_or("").starts_with("2026-09-13T10:20:30"),
            "дата - датой: {one}"
        );

        // Код ошибки в тексте: по нему ошибку и ищут.
        let err = db::query(&id, "db.runCommand({ нет_такой_команды: 1 })").await.expect_err("ошибка");
        assert!(err.contains("59"), "ожидался код 59: {err}");
        db::query(&id, "db.runCommand({ ping: 1 })").await.expect("соединение цело после ошибки");
        db::close(&id);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn mongodb_дочитывает_курсор_и_находит_пользователя_базы() {
    let s = Stand::from_env();
    rt().block_on(async {
        let id = open(&s, mongo_params(&s, "probe", "probe-pass")).await;
        db::query(&id, "db.stand_many.deleteMany({})").await.expect("очистка");
        let docs: Vec<String> = (0..1200).map(|i| format!("{{ n: {i} }}")).collect();
        db::query(&id, &format!("db.stand_many.insertMany([{}])", docs.join(", ")))
            .await
            .expect("вставка");
        // Первая пачка курсора меньше 1200: остальное приходит через getMore.
        let all = db::query(&id, "db.stand_many.find().sort({ n: 1 })").await.expect("выборка");
        assert_eq!(shown(&all)["rows"].as_array().expect("строки").len(), 1200, "курсор дочитан до конца");
        assert_eq!(all["truncated"], false);
        assert_eq!(shown(&all)["rows"][1199]["n"], "1199");

        // Учётки приложений живут в своей базе, а не в admin. Одна - со старым SCRAM-SHA-1:
        // у него своя подготовка пароля, и на обычной учётке её не проверить.
        for (user, mechanisms) in [("stand_app", "['SCRAM-SHA-256']"), ("stand_old", "['SCRAM-SHA-1']")] {
            let made = db::query(
                &id,
                &format!(
                    "db.runCommand({{ createUser: '{user}', pwd: 'app-pass', roles: [{{ role: 'readWrite', db: 'probe' }}], mechanisms: {mechanisms} }})"
                ),
            )
            .await;
            if let Err(e) = made {
                assert!(e.contains("51003"), "создание пользователя {user}: {e}");
            }
        }
        db::close(&id);

        for user in ["stand_app", "stand_old"] {
            let app = open(&s, mongo_params(&s, user, "app-pass")).await;
            let n = db::query(&app, "db.stand_many.estimatedDocumentCount()")
                .await
                .expect("запрос пользователя базы");
            assert_eq!(shown(&n)["rows"][0]["количество"], "1200", "{user}");
            db::close(&app);
        }

        let h = ssh::connect_client(vec![s.by_key(s.debian_port)])
            .await
            .expect("подключение к серверу");
        let wrong = db::open(
            format!("test-{}", uuid::Uuid::new_v4()),
            "сессия-стенда",
            &h,
            mongo_params(&s, "probe", "не-тот"),
        )
        .await
        .expect_err("неверный пароль");
        assert!(wrong.contains("18"), "ожидался код 18 (AuthenticationFailed): {wrong}");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn базы_сами_останавливают_долгий_запрос() {
    // Предел ставится самой базе, чуть меньше нашего срока: сервер, остановивший запрос сам,
    // отвечает ошибкой, соединение остаётся рабочим, и невидимой работы на нём не остаётся.
    // Проверяем, что предел выставлен, а не ждём его истечения: это полминуты на каждую базу.
    let s = Stand::from_env();
    rt().block_on(async {
        let pg = open(&s, params(Kind::Postgres, &s.pg_host, "probe", Some("probe"))).await;
        let out = db::query(&pg, "SHOW statement_timeout").await.expect("postgres");
        assert_eq!(shown(&out)["rows"][0]["statement_timeout"], "28s", "{out}");
        db::close(&pg);

        // У MariaDB и MySQL переменные разные - проверяем каждую на своей базе.
        let maria = open(&s, params(Kind::Mysql, &s.mariadb_host, "probe", Some("probe"))).await;
        let out = db::query(&maria, "SELECT @@max_statement_time AS предел").await.expect("mariadb");
        assert!(shown(&out)["rows"][0]["предел"].as_str().unwrap_or("").starts_with("28"), "{out}");
        db::close(&maria);

        let mysql = open(&s, params(Kind::Mysql, &s.mysql_host, "probe", Some("probe"))).await;
        let out = db::query(&mysql, "SELECT @@max_execution_time AS предел").await.expect("mysql");
        assert_eq!(shown(&out)["rows"][0]["предел"], "28000", "{out}");
        db::close(&mysql);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn запрос_останавливается_по_просьбе() {
    // PostgreSQL отменяет запрос, не трогая соединение. MariaDB так не умеет: соединение с
    // недочитанным ответом закрывается, и об этом сказано словами.
    let s = Stand::from_env();
    rt().block_on(async {
        let pg = open(&s, params(Kind::Postgres, &s.pg_host, "probe", Some("probe"))).await;
        let started = std::time::Instant::now();
        let running = {
            let id = pg.clone();
            tokio::spawn(async move { db::query(&id, "SELECT pg_sleep(20)").await })
        };
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        assert!(db::cancel(&pg), "соединение есть");
        let err = running.await.expect("задача").expect_err("запрос остановлен");
        assert_eq!(err, "Запрос остановлен");
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "остановка не ждёт конца запроса");
        let after = db::query(&pg, "SELECT 1 AS n").await.expect("соединение цело после отмены");
        assert_eq!(shown(&after)["rows"][0]["n"], "1");
        db::close(&pg);

        let maria = open(&s, params(Kind::Mysql, &s.mariadb_host, "probe", Some("probe"))).await;
        let running = {
            let id = maria.clone();
            tokio::spawn(async move { db::query(&id, "SELECT SLEEP(20)").await })
        };
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        assert!(db::cancel(&maria));
        let err = running.await.expect("задача").expect_err("запрос остановлен");
        assert!(err.contains("соединение с базой закрыто"), "{err}");
        assert!(db::query(&maria, "SELECT 1").await.is_err(), "соединение закрыто");
    });
}
