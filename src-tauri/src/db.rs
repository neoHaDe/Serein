//! Базы данных рядом с сервером: PostgreSQL, MySQL/MariaDB и Redis через уже открытую
//! SSH-сессию.
//!
//! Смысл ровно в слове «через». Базу почти никогда не выставляют в сеть: она слушает
//! `127.0.0.1` или внутренний адрес, и добраться до неё можно только с самого сервера.
//! Обычный путь — руками поднять проброс порта, запомнить, что он поднят, не забыть закрыть.
//! Здесь соединение открывается каналом `direct-tcpip` внутри той же сессии, по которой
//! человек и так подключён: ни открытого порта на своей машине, ни отдельной аутентификации.
//!
//! Клиенты выбраны по одному признаку — они принимают **готовый поток**, а не сами лезут
//! в сеть по адресу. Без этого канал внутрь не отдать, и пришлось бы возвращаться к пробросу.
//! У MySQL такого клиента не нашлось вовсе, поэтому его протокол разобран у нас — см.
//! [`crate::mysql`], там же объяснено, почему выбран этот путь, а не проброс порта.

use crate::ssh::SharedHandle;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;

/// Какая база на том конце. От этого зависит и порт по умолчанию, и язык запросов.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Postgres,
    /// MySQL и MariaDB — один протокол и один порт, различать их клиенту незачем.
    Mysql,
    Redis,
}

impl Kind {
    /// Порт, на котором база слушает, если не сказано иное.
    pub fn default_port(self) -> u16 {
        match self {
            Kind::Postgres => 5432,
            Kind::Mysql => 3306,
            Kind::Redis => 6379,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Postgres => "postgres",
            Kind::Mysql => "mysql",
            Kind::Redis => "redis",
        }
    }
}

/// Параметры подключения к базе — то, что приходит из формы.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Params {
    pub kind: Kind,
    /// Адрес со стороны сервера. Обычно `127.0.0.1` — база и не должна смотреть наружу.
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    /// Имя базы (PostgreSQL, MySQL) или номер базы (Redis).
    #[serde(default)]
    pub database: Option<String>,
}

impl Params {
    pub fn host(&self) -> &str {
        self.host.as_deref().filter(|h| !h.is_empty()).unwrap_or("127.0.0.1")
    }

    pub fn port(&self) -> u16 {
        self.port.filter(|p| *p != 0).unwrap_or(self.kind.default_port())
    }
}

type MysqlConn = crate::mysql::Conn<russh::ChannelStream<russh::client::Msg>>;

enum Live {
    Postgres(Arc<tokio_postgres::Client>),
    /// Под замком, а не как у PostgreSQL: наш клиент MySQL держит один поток и
    /// разговаривает по нему строго по очереди — запрос, потом ответ.
    Mysql(Arc<AsyncMutex<MysqlConn>>),
    Redis(Arc<AsyncMutex<redis::aio::MultiplexedConnection>>),
}

/// Открытое соединение вместе с тем, через какую SSH-сессию оно идёт.
///
/// Принадлежность нужна не для порядка: канал живёт внутри сессии, и когда сессия
/// закрывается, соединение с базой становится мёртвым. Без этой пометки оно осталось бы
/// в карте до конца работы приложения.
struct Open {
    session_id: String,
    live: Live,
}

static SESSIONS: Mutex<Option<HashMap<String, Open>>> = Mutex::new(None);

fn with_sessions<T>(f: impl FnOnce(&mut HashMap<String, Open>) -> T) -> T {
    let mut g = crate::sync::lock(&SESSIONS);
    f(g.get_or_insert_with(HashMap::new))
}

/// Роняет соединение внутри асинхронного рантайма.
///
/// Не блажь, а обязательное условие. `russh` в деструкторе канала вызывает
/// `tokio::spawn`, чтобы вежливо отправить серверу «закрываю». Вне рантайма этот вызов
/// паникует, а паника в деструкторе не разворачивается — процесс просто падает целиком.
/// Команда закрытия панели приходит с главного потока, поэтому ронять здесь, где придётся,
/// нельзя: одно нажатие крестика убивало бы приложение.
fn drop_in_runtime(open: Open) {
    tauri::async_runtime::spawn(async move {
        drop(open);
    });
}

/// Открывает канал до базы со стороны сервера.
async fn channel(
    handle: &SharedHandle,
    host: &str,
    port: u16,
) -> Result<russh::ChannelStream<russh::client::Msg>, String> {
    let ch = {
        let g = handle.lock().await;
        // «127.0.0.1» здесь — петля сервера, а не наша: канал открывает удалённая сторона.
        g.channel_open_direct_tcpip(host, port as u32, "127.0.0.1", 0)
            .await
            .map_err(|e| format!("Канал до {host}:{port} не открылся: {e}"))?
    };
    Ok(ch.into_stream())
}

/// Подключается к базе и запоминает соединение под выданным идентификатором.
pub async fn open(
    id: String,
    session_id: &str,
    handle: &SharedHandle,
    p: Params,
) -> Result<Value, String> {
    let stream = channel(handle, p.host(), p.port()).await?;
    let live = match p.kind {
        Kind::Postgres => {
            let mut cfg = tokio_postgres::Config::new();
            cfg.user(p.user.as_deref().unwrap_or("postgres"));
            if let Some(pw) = p.password.as_deref().filter(|s| !s.is_empty()) {
                cfg.password(pw);
            }
            if let Some(db) = p.database.as_deref().filter(|s| !s.is_empty()) {
                cfg.dbname(db);
            }
            // TLS не используем намеренно: канал уже внутри SSH, второй слой шифрования
            // здесь ничего не добавляет, зато добавил бы разбор сертификатов базы.
            let (client, conn) = cfg
                .connect_raw(stream, tokio_postgres::NoTls)
                .await
                .map_err(|e| pg_err(&e))?;
            // Соединение — это отдельная задача, которая качает байты. Без неё клиент
            // молчит: он только формирует запросы, а работает с сокетом именно она.
            tokio::spawn(async move {
                let _ = conn.await;
            });
            Live::Postgres(Arc::new(client))
        }
        Kind::Mysql => {
            let conn = crate::mysql::Conn::connect(
                stream,
                p.user.as_deref().unwrap_or("root"),
                p.password.as_deref().unwrap_or(""),
                p.database.as_deref(),
            )
            .await?;
            Live::Mysql(Arc::new(AsyncMutex::new(conn)))
        }
        Kind::Redis => {
            let mut info = redis::RedisConnectionInfo::default();
            if let Some(pw) = p.password.as_deref().filter(|s| !s.is_empty()) {
                info = info.set_password(pw);
            }
            if let Some(u) = p.user.as_deref().filter(|s| !s.is_empty()) {
                info = info.set_username(u);
            }
            if let Some(db) = p.database.as_deref().and_then(|d| d.parse::<i64>().ok()) {
                info = info.set_db(db);
            }
            // `new_with_config`, а не `new`: только у него возвращаемая задача объявлена
            // `'static`, и лишь такую можно отдать в `tokio::spawn`. У `new` она привязана
            // к ссылке на параметры и живёт не дольше этой функции.
            let (conn, driver) = redis::aio::MultiplexedConnection::new_with_config(
                &info,
                stream,
                redis::AsyncConnectionConfig::default(),
            )
            .await
            .map_err(|e| redis_err(&e))?;
            tokio::spawn(driver);
            Live::Redis(Arc::new(AsyncMutex::new(conn)))
        }
    };
    let kind = p.kind;
    with_sessions(|m| m.insert(id.clone(), Open { session_id: session_id.to_string(), live }));
    Ok(json!({ "id": id, "kind": kind.as_str(), "host": p.host(), "port": p.port() }))
}

/// Выполняет запрос и возвращает таблицу: колонки, строки и сколько это заняло.
pub async fn query(id: &str, text: &str) -> Result<Value, String> {
    let live = with_sessions(|m| match m.get(id).map(|o| &o.live) {
        Some(Live::Postgres(c)) => Some(Live::Postgres(c.clone())),
        Some(Live::Mysql(c)) => Some(Live::Mysql(c.clone())),
        Some(Live::Redis(c)) => Some(Live::Redis(c.clone())),
        None => None,
    })
    .ok_or("Соединение с базой закрыто")?;

    let started = std::time::Instant::now();
    let mut out = match live {
        Live::Postgres(c) => pg_query(&c, text).await?,
        Live::Mysql(c) => mysql_query(&c, text).await?,
        Live::Redis(c) => redis_query(&c, text).await?,
    };
    if let Some(o) = out.as_object_mut() {
        o.insert("ms".into(), json!(started.elapsed().as_millis() as u64));
    }
    Ok(out)
}

async fn pg_query(client: &tokio_postgres::Client, sql: &str) -> Result<Value, String> {
    // `simple_query` вместо подготовленных выражений: пользователь пишет произвольный текст,
    // в котором может быть несколько операторов сразу, а типов параметров тут нет вовсе.
    let msgs = client.simple_query(sql).await.map_err(|e| pg_err(&e))?;

    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<Value> = Vec::new();
    let mut affected: u64 = 0;

    for m in msgs {
        match m {
            tokio_postgres::SimpleQueryMessage::Row(r) => {
                if columns.is_empty() {
                    columns = r.columns().iter().map(|c| c.name().to_string()).collect();
                }
                let mut obj = Map::new();
                for (i, name) in columns.iter().enumerate() {
                    // NULL и пустая строка — разные вещи, и в таблице их надо различать.
                    obj.insert(name.clone(), r.get(i).map(|v| json!(v)).unwrap_or(Value::Null));
                }
                rows.push(Value::Object(obj));
            }
            tokio_postgres::SimpleQueryMessage::CommandComplete(n) => affected += n,
            _ => {}
        }
    }
    Ok(json!({ "columns": columns, "rows": rows, "affected": affected }))
}

async fn mysql_query(conn: &AsyncMutex<MysqlConn>, sql: &str) -> Result<Value, String> {
    let out = {
        let mut g = conn.lock().await;
        g.query(sql).await?
    };
    let rows: Vec<Value> = out
        .rows
        .into_iter()
        .map(|r| {
            let mut obj = Map::new();
            for (name, cell) in out.columns.iter().zip(r) {
                // NULL и пустая строка — разные вещи, как и у PostgreSQL.
                obj.insert(name.clone(), cell.map(Value::String).unwrap_or(Value::Null));
            }
            Value::Object(obj)
        })
        .collect();
    Ok(json!({ "columns": out.columns, "rows": rows, "affected": out.affected }))
}

async fn redis_query(
    conn: &AsyncMutex<redis::aio::MultiplexedConnection>,
    line: &str,
) -> Result<Value, String> {
    let parts = split_command(line);
    let Some((name, args)) = parts.split_first() else {
        return Err("Пустая команда".into());
    };
    let mut cmd = redis::cmd(name);
    for a in args {
        cmd.arg(a.as_str());
    }
    let value: redis::Value = {
        let mut g = conn.lock().await;
        cmd.query_async(&mut *g).await.map_err(|e| redis_err(&e))?
    };
    Ok(json!({ "columns": ["значение"], "rows": redis_rows(value), "affected": 0 }))
}

/// Ответ Redis — дерево, а таблица плоская. Разворачиваем список в строки, всё остальное
/// показываем одной строкой: смысл в том, чтобы результат было видно, а не в точной форме.
fn redis_rows(v: redis::Value) -> Vec<Value> {
    match v {
        redis::Value::Array(items) => items
            .into_iter()
            .map(|i| json!({ "значение": redis_scalar(i) }))
            .collect(),
        other => vec![json!({ "значение": redis_scalar(other) })],
    }
}

fn redis_scalar(v: redis::Value) -> Value {
    match v {
        redis::Value::Nil => Value::Null,
        redis::Value::Int(i) => json!(i),
        redis::Value::BulkString(b) => json!(String::from_utf8_lossy(&b).to_string()),
        redis::Value::SimpleString(s) => json!(s),
        redis::Value::Okay => json!("OK"),
        redis::Value::Double(d) => json!(d),
        redis::Value::Boolean(b) => json!(b),
        other => json!(format!("{other:?}")),
    }
}

/// Разбирает строку команды Redis, уважая кавычки: `SET ключ "два слова"` — три части.
pub fn split_command(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;

    for ch in line.trim().chars() {
        match ch {
            '"' | '\'' if quote.is_none() => {
                quote = Some(ch);
                has = true;
            }
            c if Some(c) == quote => quote = None,
            c if c.is_whitespace() && quote.is_none() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn close(id: &str) {
    if let Some(open) = with_sessions(|m| m.remove(id)) {
        drop_in_runtime(open);
    }
}

/// Закрывает все базы, открытые через указанную SSH-сессию.
///
/// Их каналы живут внутри неё, и пережить её они не могут — а вот остаться в карте
/// мёртвыми вполне. Тогда следующий запрос уходил бы в никуда и ждал ответа.
pub fn close_session(session_id: &str) {
    let gone: Vec<Open> = with_sessions(|m| {
        let ids: Vec<String> = m
            .iter()
            .filter(|(_, o)| o.session_id == session_id)
            .map(|(k, _)| k.clone())
            .collect();
        ids.iter().filter_map(|k| m.remove(k)).collect()
    });
    for open in gone {
        drop_in_runtime(open);
    }
}

/// Сколько соединений открыто через эту сессию — для тестов и диагностики.
pub fn count_for_session(session_id: &str) -> usize {
    with_sessions(|m| m.values().filter(|o| o.session_id == session_id).count())
}

fn pg_err(e: &tokio_postgres::Error) -> String {
    // У ошибки базы есть человеческое сообщение внутри; без него наружу уходит
    // «db error» без единой подробности.
    match e.as_db_error() {
        Some(db) => format!("{}: {}", db.code().code(), db.message()),
        None => e.to_string(),
    }
}

fn redis_err(e: &redis::RedisError) -> String {
    match e.detail() {
        Some(d) => format!("{}: {d}", e.category()),
        None => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn порт_берётся_по_виду_базы_если_не_задан() {
        let p = Params {
            kind: Kind::Postgres,
            host: None,
            port: None,
            user: None,
            password: None,
            database: None,
        };
        assert_eq!(p.port(), 5432);
        // Пустой хост — это «не указано», а не адрес: подставляем петлю сервера.
        assert_eq!(p.host(), "127.0.0.1");
    }

    #[test]
    fn нулевой_порт_считается_незаданным() {
        // Форма отдаёт 0, когда поле очищено. Подключаться на нулевой порт бессмысленно.
        let p = Params {
            kind: Kind::Redis,
            host: Some(String::new()),
            port: Some(0),
            user: None,
            password: None,
            database: None,
        };
        assert_eq!(p.port(), 6379);
        assert_eq!(p.host(), "127.0.0.1");
    }

    #[test]
    fn команда_redis_режется_по_пробелам() {
        assert_eq!(split_command("GET ключ"), vec!["GET", "ключ"]);
        assert_eq!(split_command("  PING  "), vec!["PING"]);
        assert!(split_command("   ").is_empty());
    }

    #[test]
    fn кавычки_держат_значение_целиком() {
        // Иначе «два слова» превратятся в два аргумента, и запись уйдёт обрезанной.
        assert_eq!(
            split_command(r#"SET ключ "два слова""#),
            vec!["SET", "ключ", "два слова"]
        );
        assert_eq!(split_command("SET k 'одинарные тоже'"), vec!["SET", "k", "одинарные тоже"]);
    }

    #[test]
    fn пустая_строка_в_кавычках_остаётся_аргументом() {
        // `SET ключ ""` — законная запись пустого значения, и терять её нельзя.
        assert_eq!(split_command(r#"SET ключ """#), vec!["SET", "ключ", ""]);
    }
}
