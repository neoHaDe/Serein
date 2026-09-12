//! Базы данных рядом с сервером: PostgreSQL, MySQL/MariaDB и Redis через уже открытую
//! SSH-сессию.
//!
//! Смысл ровно в слове «через». Базу почти никогда не выставляют в сеть: она слушает
//! `127.0.0.1` или внутренний адрес, и добраться до неё можно только с самого сервера.
//! Обычный путь - руками поднять проброс порта, запомнить, что он поднят, не забыть закрыть.
//! Здесь соединение открывается каналом `direct-tcpip` внутри той же сессии, по которой
//! человек и так подключён: ни открытого порта на своей машине, ни отдельной аутентификации.
//!
//! Клиенты выбраны по одному признаку - они принимают **готовый поток**, а не сами лезут
//! в сеть по адресу. Без этого канал внутрь не отдать, и пришлось бы возвращаться к пробросу.
//! У MySQL такого клиента не нашлось вовсе, поэтому его протокол разобран у нас - см.
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
    /// MySQL и MariaDB - один протокол и один порт, различать их клиенту незачем.
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

/// Параметры подключения к базе - то, что приходит из формы.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Params {
    pub kind: Kind,
    /// Адрес со стороны сервера. Обычно `127.0.0.1` - база и не должна смотреть наружу.
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
    /// разговаривает по нему строго по очереди - запрос, потом ответ.
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
    /// Что показать в шапке: вид базы, адрес и порт. Хранится здесь, потому что панель
    /// в откреплённом окне - это отдельный веб-контекст, и своей памяти о соединении
    /// у неё нет. Спросить она может только приложение.
    info: Value,
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
/// паникует, а паника в деструкторе не разворачивается - процесс просто падает целиком.
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
    let ch = crate::ssh::open_forward_channel(handle, host, port).await?;
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
            // Соединение - это отдельная задача, которая качает байты. Без неё клиент
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
    let info = json!({ "id": id, "kind": kind.as_str(), "host": p.host(), "port": p.port() });
    with_sessions(|m| {
        m.insert(
            id.clone(),
            Open { session_id: session_id.to_string(), info: info.clone(), live },
        )
    });
    Ok(info)
}

/// Пределы одного запроса.
///
/// Они не про вкус к аккуратности. `SELECT * FROM logs` на живой базе - это миллионы
/// строк: они целиком приезжают в память приложения, оттуда в JSON, оттуда в таблицу
/// интерфейса, и приложение встаёт намертво на машине, где база отвечала мгновенно.
/// Поэтому берём столько, сколько имеет смысл показывать человеку, и честно говорим, что
/// показано не всё.
const MAX_ROWS: usize = 5_000;
/// Грубая мера объёма ответа: сумма длин значений. Пять тысяч строк по мегабайту - тоже
/// способ убить приложение.
const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Одна ячейка. Столбец с картинкой или документом показывать целиком незачем.
const MAX_CELL: usize = 64 * 1024;
/// Сколько ждём ответа. Дальше соединение закрывается: продолжать по нему нельзя.
const QUERY_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Делает имена колонок различимыми.
///
/// `SELECT 1 AS a, 2 AS a` возвращает две колонки с одним именем, а строка в ответе -
/// словарь: второе значение затирало первое, и человек видел таблицу, где одного столбца
/// просто нет. Повторы получают номер.
fn unique_names<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        let mut candidate = name.to_owned();
        let mut n = 2;
        while out.contains(&candidate) {
            candidate = format!("{name}#{n}");
            n += 1;
        }
        out.push(candidate);
    }
    out
}

/// Один набор результатов: свои колонки, свои строки, своё число изменённых.
struct ResultSet {
    columns: Vec<String>,
    budget: Budget,
    affected: u64,
}

impl ResultSet {
    fn new(columns: Vec<String>) -> Self {
        Self { columns, budget: Budget::new(), affected: 0 }
    }

    fn json(self) -> Value {
        json!({
            "columns": self.columns,
            "rows": self.budget.rows,
            "affected": self.affected,
            "truncated": self.budget.truncated,
        })
    }
}

/// Собирает ответ из наборов.
///
/// В таблицу идёт первый набор со строками - обычно его и ждут, когда пишут несколько
/// операторов подряд. Остальные не выбрасываются: они уходят в `sets`, и панель даёт
/// переключиться. Прежний разбор запоминал колонки первой строки и применял их ко всем
/// следующим наборам - на `SELECT 1 AS a, 2 AS b; SELECT 3 AS c` это означало обращение к
/// колонке, которой в строке нет, а закреплённая библиотека на таком обращении паникует.
fn answer(sets: Vec<ResultSet>) -> Value {
    let affected: u64 = sets.iter().map(|s| s.affected).sum();
    let truncated = sets.iter().any(|s| s.budget.truncated);
    let sets: Vec<Value> = sets.into_iter().map(ResultSet::json).collect();
    let shown = sets
        .iter()
        .position(|s| !s["rows"].as_array().map(|r| r.is_empty()).unwrap_or(true))
        .unwrap_or(0);
    let head = sets.get(shown).cloned().unwrap_or_else(|| json!({}));
    json!({
        "columns": head.get("columns").cloned().unwrap_or_else(|| json!([])),
        "rows": head.get("rows").cloned().unwrap_or_else(|| json!([])),
        "affected": affected,
        "truncated": truncated,
        "sets": sets,
        "shown": shown,
    })
}

/// Складывает строки, пока они укладываются в пределы.
///
/// Общая для всех трёх баз: пределы обязаны быть одинаковыми, иначе «много строк» в одной
/// панели значит одно, а в соседней другое.
struct Budget {
    rows: Vec<Value>,
    bytes: usize,
    truncated: bool,
}

impl Budget {
    fn new() -> Self {
        Self { rows: Vec::new(), bytes: 0, truncated: false }
    }

    /// Обрезает слишком длинное значение, сообщая об этом в самом значении.
    fn cell(&mut self, v: &str) -> Value {
        if v.len() <= MAX_CELL {
            return json!(v);
        }
        self.truncated = true;
        let mut cut = MAX_CELL;
        // Режем по границе символа: иначе в таблицу уедет битый UTF-8.
        while cut > 0 && !v.is_char_boundary(cut) {
            cut -= 1;
        }
        json!(format!("{}… (обрезано, всего {} Б)", &v[..cut], v.len()))
    }

    /// `false` - место кончилось, дальше складывать нечего.
    fn push(&mut self, row: Value, size: usize) -> bool {
        if self.rows.len() >= MAX_ROWS || self.bytes.saturating_add(size) > MAX_BYTES {
            self.truncated = true;
            return false;
        }
        self.bytes += size;
        self.rows.push(row);
        true
    }
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
    let work = async {
        match live {
            Live::Postgres(c) => pg_query(&c, text).await,
            Live::Mysql(c) => mysql_query(&c, text).await,
            Live::Redis(c) => redis_query(&c, text).await,
        }
    };
    // Срок на запрос, и по его истечении соединение закрывается. Это не перестраховка:
    // брошенный на середине запрос оставляет протокол в неизвестном состоянии, и
    // следующий запрос по тому же соединению прочтёт хвост предыдущего - выглядеть это
    // будет как «база вернула ерунду».
    let mut out = match tokio::time::timeout(QUERY_LIMIT, work).await {
        Ok(r) => r?,
        Err(_) => {
            close(id);
            return Err(format!(
                "запрос не ответил за {} с - соединение закрыто, откройте его заново",
                QUERY_LIMIT.as_secs()
            ));
        }
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

    let mut sets: Vec<ResultSet> = Vec::new();
    let mut current: Option<ResultSet> = None;

    for m in msgs {
        match m {
            // Описание колонок начинает новый набор - это и есть граница между выборками.
            tokio_postgres::SimpleQueryMessage::RowDescription(cols) => {
                if let Some(done) = current.take() {
                    sets.push(done);
                }
                current = Some(ResultSet::new(unique_names(cols.iter().map(|c| c.name()))));
            }
            tokio_postgres::SimpleQueryMessage::Row(r) => {
                // Колонки берём из самой строки, если описания не было: обращаться по
                // индексу, которого в строке нет, нельзя - библиотека на этом паникует.
                let set = current.get_or_insert_with(|| {
                    ResultSet::new(unique_names(r.columns().iter().map(|c| c.name())))
                });
                let mut obj = Map::new();
                let mut size = 0usize;
                for (i, name) in set.columns.iter().enumerate() {
                    if i >= r.len() {
                        obj.insert(name.clone(), Value::Null);
                        continue;
                    }
                    // NULL и пустая строка - разные вещи, и в таблице их надо различать.
                    let cell = match r.get(i) {
                        Some(v) => {
                            size += v.len();
                            set.budget.cell(v)
                        }
                        None => Value::Null,
                    };
                    obj.insert(name.clone(), cell);
                }
                if !set.budget.push(Value::Object(obj), size) {
                    // Место кончилось - остальные строки этого набора не берём, но
                    // следующие наборы разобрать обязаны.
                    continue;
                }
            }
            tokio_postgres::SimpleQueryMessage::CommandComplete(n) => {
                let mut set = current.take().unwrap_or_else(|| ResultSet::new(Vec::new()));
                set.affected = n;
                sets.push(set);
            }
            _ => {}
        }
    }
    if let Some(done) = current.take() {
        sets.push(done);
    }
    Ok(answer(sets))
}

async fn mysql_query(conn: &AsyncMutex<MysqlConn>, sql: &str) -> Result<Value, String> {
    let outs = {
        let mut g = conn.lock().await;
        g.query(sql).await?
    };
    let mut sets: Vec<ResultSet> = Vec::new();
    for out in outs {
        let mut set = ResultSet::new(unique_names(out.columns.iter().map(String::as_str)));
        set.affected = out.affected;
        for r in out.rows {
            let mut obj = Map::new();
            let mut size = 0usize;
            for (name, cell) in set.columns.iter().zip(r) {
                // NULL и пустая строка - разные вещи, как и у PostgreSQL.
                let value = match cell {
                    Some(v) => {
                        size += v.len();
                        set.budget.cell(&v)
                    }
                    None => Value::Null,
                };
                obj.insert(name.clone(), value);
            }
            if !set.budget.push(Value::Object(obj), size) {
                break;
            }
        }
        sets.push(set);
    }
    Ok(answer(sets))
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
    let mut set = ResultSet::new(vec!["значение".to_owned()]);
    for row in redis_rows(value) {
        let text = row["значение"].as_str().unwrap_or("").to_owned();
        let size = text.len();
        let cell = if text.is_empty() { row["значение"].clone() } else { set.budget.cell(&text) };
        if !set.budget.push(json!({ "значение": cell }), size) {
            break;
        }
    }
    Ok(answer(vec![set]))
}

/// Ответ Redis - дерево, а таблица плоская. Разворачиваем список в строки, всё остальное
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

/// Разбирает строку команды Redis, уважая кавычки: `SET ключ "два слова"` - три части.
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
/// Их каналы живут внутри неё, и пережить её они не могут - а вот остаться в карте
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

/// Открытая база этой сессии, если она есть.
///
/// Нужна, когда панель появляется на пустом месте и не помнит ничего: откреплённое окно
/// - это отдельный веб-контекст со своей памятью, а соединение живёт в приложении и
/// переезд окна переживает. Без этого вопроса отделение панели выглядело бы как обрыв
/// связи, хотя рвать было нечего.
pub fn for_session(session_id: &str) -> Option<Value> {
    with_sessions(|m| {
        m.values()
            .find(|o| o.session_id == session_id)
            .map(|o| o.info.clone())
    })
}

/// Сколько соединений открыто через эту сессию - для тестов и диагностики.
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
    fn одинаковые_имена_колонок_не_съедают_друг_друга() {
        // `SELECT 1 AS a, 2 AS a` - строка в ответе словарь, и второе значение затирало
        // первое: в таблице просто не было одного столбца.
        assert_eq!(unique_names(["a", "b", "a", "a"].into_iter()), vec!["a", "b", "a#2", "a#3"]);
        assert_eq!(unique_names([].into_iter()), Vec::<String>::new());
    }

    #[test]
    fn показывается_первый_набор_со_строками() {
        // Несколько операторов подряд: данные часто приходят не первым набором, а вторым.
        // Раньше показывался первый, и человек видел пустую таблицу без объяснений.
        let mut пустой = ResultSet::new(vec!["a".into()]);
        пустой.affected = 3;
        let mut со_строками = ResultSet::new(vec!["c".into()]);
        со_строками.budget.push(json!({ "c": 3 }), 1);
        со_строками.affected = 1;

        let ответ = answer(vec![пустой, со_строками]);
        assert_eq!(ответ["shown"], 1, "показать надо набор со строками");
        assert_eq!(ответ["columns"][0], "c");
        assert_eq!(ответ["sets"].as_array().unwrap().len(), 2, "остальные наборы не выбрасываются");
        assert_eq!(ответ["affected"], 4, "изменённые строки складываются по всем наборам");
    }

    #[test]
    fn выборка_обрезается_по_строкам_объёму_и_ячейке() {
        // Без этих пределов `SELECT * FROM logs` на живой базе приезжает целиком: в память,
        // в JSON, в таблицу - и приложение встаёт на машине, где база отвечала мгновенно.
        let mut по_строкам = Budget::new();
        for i in 0..MAX_ROWS {
            assert!(по_строкам.push(json!({ "i": i }), 1), "строка {i} должна поместиться");
        }
        assert!(!по_строкам.push(json!({ "i": "лишняя" }), 1), "предел строк обязан сработать");
        assert!(по_строкам.truncated, "об обрезке надо сказать");

        let mut по_объёму = Budget::new();
        assert!(!по_объёму.push(json!({ "a": 1 }), MAX_BYTES + 1), "предел объёма обязан сработать");
        assert!(по_объёму.truncated);

        let mut по_ячейке = Budget::new();
        // Кириллица - два байта на символ: длинная строка режется по границе символа, иначе
        // в таблицу уедет битый UTF-8.
        let длинная = "я".repeat(MAX_CELL);
        let обрезанная = по_ячейке.cell(&длинная);
        let текст = обрезанная.as_str().expect("значение - строка");
        assert!(текст.contains("обрезано"), "обрезка обязана быть видна: {текст:.40}");
        assert!(текст.len() < длинная.len());
        assert!(по_ячейке.truncated);

        // Короткое значение проходит как есть, без пометок.
        let mut целое = Budget::new();
        assert_eq!(целое.cell("значение"), json!("значение"));
        assert!(!целое.truncated);
    }

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
        // Пустой хост - это «не указано», а не адрес: подставляем петлю сервера.
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
        // `SET ключ ""` - законная запись пустого значения, и терять её нельзя.
        assert_eq!(split_command(r#"SET ключ """#), vec!["SET", "ключ", ""]);
    }
}
