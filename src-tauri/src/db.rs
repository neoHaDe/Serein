//! Базы данных рядом с сервером: PostgreSQL, MySQL/MariaDB, SQL Server, SQLite, MongoDB и Redis через уже открытую
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
//! С MongoDB то же самое - см. [`crate::mongo`].

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
    /// Microsoft SQL Server.
    Mssql,
    /// SQLite: файл на сервере, запросы через `sqlite3` там же.
    Sqlite,
    /// MongoDB: свой протокол поверх канала, см. [`crate::mongo`].
    Mongo,
    Redis,
}

impl Kind {
    /// Порт, на котором база слушает, если не сказано иное.
    pub fn default_port(self) -> u16 {
        match self {
            Kind::Postgres => 5432,
            Kind::Mysql => 3306,
            Kind::Mssql => 1433,
            // У SQLite порта нет: это файл, а не служба.
            Kind::Sqlite => 0,
            Kind::Mongo => 27017,
            Kind::Redis => 6379,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Postgres => "postgres",
            Kind::Mysql => "mysql",
            Kind::Mssql => "mssql",
            Kind::Sqlite => "sqlite",
            Kind::Mongo => "mongo",
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
type MongoConn = crate::mongo::Conn<russh::ChannelStream<russh::client::Msg>>;

/// PostgreSQL вместе с тем, что нужно для отмены запроса: протокол отменяет запрос
/// отдельным коротким соединением, и открывать его приходится тем же каналом внутри SSH.
struct PgLive {
    client: tokio_postgres::Client,
    cancel: tokio_postgres::CancelToken,
    handle: SharedHandle,
    host: String,
    port: u16,
}

enum Live {
    Postgres(Arc<PgLive>),
    /// Под замком, а не как у PostgreSQL: наш клиент MySQL держит один поток и
    /// разговаривает по нему строго по очереди - запрос, потом ответ.
    Mysql(Arc<AsyncMutex<MysqlConn>>),
    /// Под замком: запросы tiberius требуют исключительного доступа к клиенту и идут строго
    /// по очереди, как у нашего клиента MySQL.
    Mssql(Arc<AsyncMutex<MssqlClient>>),
    /// У SQLite держать открытым нечего: каждый запрос - отдельный запуск `sqlite3`.
    Sqlite(Arc<SqliteTarget>),
    /// Под замком: один поток, запрос и ответ строго по очереди, как у MySQL.
    Mongo(Arc<AsyncMutex<MongoConn>>),
    Redis(Arc<AsyncMutex<redis::aio::MultiplexedConnection>>),
}

/// Клиент SQL Server поверх канала SSH. Переходник нужен потому, что tiberius говорит на
/// потоках futures, а канал - поток tokio.
type MssqlClient =
    tiberius::Client<tokio_util::compat::Compat<russh::ChannelStream<russh::client::Msg>>>;

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
    /// Просьба остановить запрос, который сейчас выполняется. Отдельно от `live`: запрос
    /// держит свою копию соединения, и убрать соединение из карты его не остановит.
    stop: Arc<tokio::sync::Notify>,
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
    // Канал внутри SSH нужен сетевым базам. SQLite - файл, до него канал не открываем.
    let live = match p.kind {
        Kind::Postgres => {
            let stream = channel(handle, p.host(), p.port()).await?;
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
            // Предел на стороне сервера - см. SERVER_LIMIT. Прав `SET` не требует, но пул
            // соединений вроде PgBouncer в режиме транзакций его не удержит - тогда остаётся
            // один наш срок, как было раньше.
            let _ = client
                .simple_query(&format!("SET statement_timeout = {}", SERVER_LIMIT.as_millis()))
                .await;
            let cancel = client.cancel_token();
            Live::Postgres(Arc::new(PgLive {
                client,
                cancel,
                handle: handle.clone(),
                host: p.host().to_owned(),
                port: p.port(),
            }))
        }
        Kind::Mysql => {
            let stream = channel(handle, p.host(), p.port()).await?;
            let mut conn = crate::mysql::Conn::connect(
                stream,
                p.user.as_deref().unwrap_or("root"),
                p.password.as_deref().unwrap_or(""),
                p.database.as_deref(),
            )
            .await?;
            // Предел на стороне сервера - см. SERVER_LIMIT. У MariaDB и MySQL переменные
            // разные, и каждая неизвестна другой базе: ставим обе, чужая просто вернёт
            // ошибку. У MySQL предел действует только на SELECT - изменения он не прерывает.
            let _ = conn
                .query(&format!("SET SESSION max_statement_time = {}", SERVER_LIMIT.as_secs()))
                .await;
            let _ = conn
                .query(&format!("SET SESSION max_execution_time = {}", SERVER_LIMIT.as_millis()))
                .await;
            Live::Mysql(Arc::new(AsyncMutex::new(conn)))
        }
        Kind::Redis => {
            let stream = channel(handle, p.host(), p.port()).await?;
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
        Kind::Mssql => {
            let stream = channel(handle, p.host(), p.port()).await?;
            use tokio_util::compat::TokioAsyncWriteCompatExt as _;
            let mut cfg = tiberius::Config::new();
            cfg.host(p.host());
            cfg.port(p.port());
            cfg.authentication(tiberius::AuthMethod::sql_server(
                p.user.as_deref().filter(|s| !s.is_empty()).unwrap_or("sa"),
                p.password.as_deref().unwrap_or(""),
            ));
            if let Some(db) = p.database.as_deref().filter(|s| !s.is_empty()) {
                cfg.database(db);
            }
            // Шифрование включаем, хотя канал и так внутри SSH: SQL Server с обязательным
            // шифрованием иначе просто закроет соединение, а у новых установок оно такое по
            // умолчанию. Сертификат не проверяем по той же причине, что у RDP: подлинность
            // стороны даёт SSH-сессия, а сертификаты SQL Server почти всегда самоподписанные.
            cfg.encryption(tiberius::EncryptionLevel::Required);
            cfg.trust_cert();
            let client = tiberius::Client::connect(cfg, stream.compat_write())
                .await
                .map_err(|e| mssql_err(&e))?;
            Live::Mssql(Arc::new(AsyncMutex::new(client)))
        }
        Kind::Mongo => {
            let stream = channel(handle, p.host(), p.port()).await?;
            let conn = crate::mongo::Conn::connect(
                stream,
                p.user.as_deref().unwrap_or(""),
                p.password.as_deref().unwrap_or(""),
                p.database.as_deref().unwrap_or(""),
            )
            .await?;
            Live::Mongo(Arc::new(AsyncMutex::new(conn)))
        }
        Kind::Sqlite => Live::Sqlite(Arc::new(sqlite_open(handle, &p).await?)),
    };
    let kind = p.kind;
    // У SQLite вместо адреса - путь к файлу: по нему человек и узнает базу в заголовке.
    let place = if kind == Kind::Sqlite {
        p.database.clone().unwrap_or_default()
    } else {
        p.host().to_owned()
    };
    let info = json!({ "id": id, "kind": kind.as_str(), "host": place, "port": p.port() });
    with_sessions(|m| {
        m.insert(
            id.clone(),
            Open {
                session_id: session_id.to_string(),
                info: info.clone(),
                live,
                stop: Arc::new(tokio::sync::Notify::new()),
            },
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
/// Предел, который ставим самой базе, - чуть меньше своего срока.
///
/// Сервер, остановивший запрос сам, отвечает ошибкой, и соединение остаётся рабочим. Наш
/// срок срабатывает, только если сервер не ответил и тогда, - и соединение закрывается. Без
/// предела на сервере брошенный нами запрос продолжал бы работать там невидимо: PostgreSQL
/// замечает закрытое соединение, лишь когда пытается отправить результат.
///
/// Есть у PostgreSQL, MySQL, MariaDB и у чтения MongoDB. У SQL Server предела на сеанс нет,
/// у Redis команды не прерываются вовсе, у SQLite запрос живёт в процессе `sqlite3`.
const SERVER_LIMIT: std::time::Duration = std::time::Duration::from_secs(28);

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

/// Запрос остановлен по просьбе, соединение цело.
const STOPPED: &str = "Запрос остановлен";
/// Запрос остановлен по просьбе, но соединение пришлось закрыть. Слова «соединение с базой
/// закрыто» панель узнаёт и возвращается к форме подключения.
const STOPPED_CLOSED: &str =
    "Запрос остановлен - соединение с базой закрыто: его ответ остался недочитанным. Подключитесь заново";

/// Останавливает запрос, который выполняется на этом соединении прямо сейчас.
///
/// Будит только тех, кто уже ждёт: запрос, начатый после нажатия, остановка не заденет.
/// Иначе запоздалый щелчок отменил бы следующий, ни в чём не повинный запрос.
pub fn cancel(id: &str) -> bool {
    match with_sessions(|m| m.get(id).map(|o| o.stop.clone())) {
        Some(stop) => {
            stop.notify_waiters();
            true
        }
        None => false,
    }
}

/// Выполняет запрос и возвращает таблицу: колонки, строки и сколько это заняло.
pub async fn query(id: &str, text: &str) -> Result<Value, String> {
    let live = with_sessions(|m| match m.get(id).map(|o| &o.live) {
        Some(Live::Postgres(c)) => Some(Live::Postgres(c.clone())),
        Some(Live::Mysql(c)) => Some(Live::Mysql(c.clone())),
        Some(Live::Mssql(c)) => Some(Live::Mssql(c.clone())),
        Some(Live::Sqlite(t)) => Some(Live::Sqlite(t.clone())),
        Some(Live::Mongo(c)) => Some(Live::Mongo(c.clone())),
        Some(Live::Redis(c)) => Some(Live::Redis(c.clone())),
        None => None,
    })
    .ok_or("Соединение с базой закрыто")?;
    let stop = with_sessions(|m| m.get(id).map(|o| o.stop.clone())).ok_or("Соединение с базой закрыто")?;

    let pg = match &live {
        Live::Postgres(c) => Some(c.clone()),
        _ => None,
    };
    let started = std::time::Instant::now();
    let work = async {
        match live {
            Live::Postgres(c) => pg_query(&c.client, text).await,
            Live::Mysql(c) => mysql_query(&c, text).await,
            Live::Mssql(c) => mssql_query(&c, text).await,
            Live::Sqlite(t) => sqlite_query(&t, text).await,
            Live::Mongo(c) => mongo_query(&c, text).await,
            Live::Redis(c) => redis_query(&c, text).await,
        }
    };
    // Срок на запрос, и по его истечении соединение закрывается. Это не перестраховка:
    // брошенный на середине запрос оставляет протокол в неизвестном состоянии, и
    // следующий запрос по тому же соединению прочтёт хвост предыдущего - выглядеть это
    // будет как «база вернула ерунду». По той же причине закрывается и остановка по кнопке.
    tokio::pin!(work);
    let limit = tokio::time::sleep(QUERY_LIMIT);
    tokio::pin!(limit);
    let mut out = tokio::select! {
        r = &mut work => r?,
        _ = &mut limit => {
            // Прежде чем бросить соединение, просим сервер остановить запрос: иначе он
            // доработал бы там, где его результат уже никто не прочтёт.
            if let Some(pg) = &pg {
                pg_cancel(pg).await;
            }
            close(id);
            return Err(format!(
                "Запрос не ответил за {} с - соединение с базой закрыто, подключитесь заново",
                QUERY_LIMIT.as_secs()
            ));
        }
        _ = stop.notified() => {
            // PostgreSQL останавливает запрос, не трогая соединение: сервер отвечает ошибкой
            // 57014, её и дожидаемся. Остальные базы так не умеют, и соединение с
            // недочитанным ответом приходится закрыть.
            let Some(pg) = &pg else {
                close(id);
                return Err(STOPPED_CLOSED.into());
            };
            pg_cancel(pg).await;
            match tokio::time::timeout(std::time::Duration::from_secs(5), &mut work).await {
                // Запрос успел закончиться сам - результат честный, отдаём его.
                Ok(Ok(v)) => v,
                Ok(Err(e)) if e.starts_with("57014") => return Err(STOPPED.into()),
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    close(id);
                    return Err(STOPPED_CLOSED.into());
                }
            }
        }
    };
    if let Some(o) = out.as_object_mut() {
        o.insert("ms".into(), json!(started.elapsed().as_millis() as u64));
    }
    Ok(out)
}

/// Просьба к PostgreSQL остановить текущий запрос. Протокол шлёт её отдельным коротким
/// соединением - у нас это новый канал внутри той же SSH-сессии. Не вышло - молчим:
/// соединение всё равно закрывается, а предел на сервере остановит запрос сам.
async fn pg_cancel(pg: &PgLive) {
    let attempt = async {
        let stream = channel(&pg.handle, &pg.host, pg.port).await.ok()?;
        pg.cancel.cancel_query_raw(stream, tokio_postgres::NoTls).await.ok()
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), attempt).await;
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

/// Запрос к SQL Server. Граница набора - описание колонок, как у PostgreSQL; строки
/// складываются в те же пределы.
///
/// Поток ответа дочитываем до конца даже после того, как место в таблице кончилось: иначе
/// следующий запрос по этому соединению прочтёт хвост предыдущего.
async fn mssql_query(conn: &AsyncMutex<MssqlClient>, sql: &str) -> Result<Value, String> {
    use futures::TryStreamExt as _;
    let mut g = conn.lock().await;
    let mut stream = g.simple_query(sql).await.map_err(|e| mssql_err(&e))?;
    let mut sets: Vec<ResultSet> = Vec::new();
    let mut current: Option<ResultSet> = None;
    while let Some(item) = stream.try_next().await.map_err(|e| mssql_err(&e))? {
        match item {
            tiberius::QueryItem::Metadata(meta) => {
                if let Some(done) = current.take() {
                    sets.push(done);
                }
                current = Some(ResultSet::new(unique_names(
                    meta.columns().iter().map(|c| c.name()),
                )));
            }
            tiberius::QueryItem::Row(row) => {
                let Some(set) = current.as_mut() else {
                    continue;
                };
                let values: Vec<Option<String>> = row.into_iter().map(|c| mssql_cell(&c)).collect();
                let mut obj = Map::new();
                let mut size = 0usize;
                for (i, name) in set.columns.iter().enumerate() {
                    // NULL и пустая строка - разные вещи, как и у остальных баз.
                    let cell = match values.get(i).cloned().flatten() {
                        Some(v) => {
                            size += v.len();
                            set.budget.cell(&v)
                        }
                        None => Value::Null,
                    };
                    obj.insert(name.clone(), cell);
                }
                // Место кончилось - строку не берём, но поток дочитываем.
                let _ = set.budget.push(Value::Object(obj), size);
            }
        }
    }
    if let Some(done) = current.take() {
        sets.push(done);
    }
    Ok(answer(sets))
}

/// Значение ячейки SQL Server текстом. `None` - это NULL.
fn mssql_cell(data: &tiberius::ColumnData<'static>) -> Option<String> {
    use tiberius::ColumnData as D;
    use tiberius::FromSql as _;
    match data {
        D::U8(v) => v.map(|x| x.to_string()),
        D::I16(v) => v.map(|x| x.to_string()),
        D::I32(v) => v.map(|x| x.to_string()),
        D::I64(v) => v.map(|x| x.to_string()),
        D::F32(v) => v.map(|x| x.to_string()),
        D::F64(v) => v.map(|x| x.to_string()),
        // Как пишет сам SQL Server: 1 и 0, а не true и false.
        D::Bit(v) => v.map(|x| (if x { "1" } else { "0" }).to_owned()),
        D::String(v) => v.as_ref().map(|s| s.to_string()),
        D::Guid(v) => v.map(|g| g.to_string().to_uppercase()),
        D::Binary(v) => v.as_ref().map(|b| {
            let hex: String = b.iter().map(|x| format!("{x:02X}")).collect();
            format!("0x{hex}")
        }),
        D::Numeric(v) => v.map(|n| n.to_string()),
        D::Xml(v) => v.as_ref().map(|x| x.to_string()),
        D::DateTime(_) | D::SmallDateTime(_) | D::DateTime2(_) => {
            chrono::NaiveDateTime::from_sql(data).ok().flatten().map(|d| d.to_string())
        }
        D::Date(_) => chrono::NaiveDate::from_sql(data).ok().flatten().map(|d| d.to_string()),
        D::Time(_) => chrono::NaiveTime::from_sql(data).ok().flatten().map(|d| d.to_string()),
        D::DateTimeOffset(_) => chrono::DateTime::<chrono::FixedOffset>::from_sql(data)
            .ok()
            .flatten()
            .map(|d| d.to_string()),
    }
}

/// Текст ошибки SQL Server. У ошибок сервера есть номер - его и показываем: по номеру
/// ошибку находят в документации, а текст сообщения бывает переведён.
fn mssql_err(e: &tiberius::error::Error) -> String {
    match e {
        tiberius::error::Error::Server(t) => format!("SQL Server {}: {}", t.code(), t.message()),
        other => format!("SQL Server: {other}"),
    }
}

/// Куда ходить за SQLite: сессия и путь к файлу на сервере.
struct SqliteTarget {
    handle: SharedHandle,
    path: String,
}

/// Проверяет, что до файла SQLite можно достучаться: `sqlite3` на сервере есть, файл есть.
///
/// Новую базу не создаём: `sqlite3` молча создаёт пустой файл по любому пути, и опечатка в
/// пути превратилась бы в новую пустую базу вместо ошибки.
async fn sqlite_open(handle: &SharedHandle, p: &Params) -> Result<SqliteTarget, String> {
    let path = p
        .database
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Укажите путь к файлу базы на сервере".to_owned())?
        .to_owned();
    let q = crate::scp::shell_quote(&path);
    let cmd = format!(
        "command -v sqlite3 >/dev/null 2>&1 && echo tool; test -f {q} && echo file; test -r {q} && echo read"
    );
    let (_c, out, _e) = crate::ssh::exec(handle, &cmd, None).await?;
    let has = |w: &str| out.lines().any(|l| l.trim() == w);
    if !has("tool") {
        return Err("На сервере нет sqlite3 - поставьте пакет sqlite3 (на Alpine - sqlite)".into());
    }
    if !has("file") {
        return Err(format!(
            "Файла {path} нет - новую базу не создаём, чтобы опечатка в пути не стала пустой базой"
        ));
    }
    if !has("read") {
        return Err(format!("Файл {path} не читается под этим пользователем"));
    }
    Ok(SqliteTarget {
        handle: handle.clone(),
        path,
    })
}

/// Запрос к SQLite через `sqlite3` на самом сервере.
///
/// SQL уходит на стандартный вход, а не доводом команды: так нет ни экранирования, ни предела
/// длины строки запуска, и текст запроса не виден в списке процессов сервера. Вывод
/// обрезается там же, на сервере: иначе `SELECT *` по большой таблице приехал бы целиком.
async fn sqlite_query(t: &SqliteTarget, sql: &str) -> Result<Value, String> {
    let limit = MAX_BYTES + 1;
    let cmd = format!(
        "sqlite3 -json -bail {} | head -c {limit}",
        crate::scp::shell_quote(&t.path)
    );
    let (_code, out, err) = crate::ssh::exec_with_input(&t.handle, &cmd, sql, None).await?;
    let err = err.trim();
    if !err.is_empty() {
        // Вывод JSON появился в sqlite3 3.33 (2020). Старый sqlite3 ругается на ключ - говорим
        // по-человечески, что именно не так, а не пересказываем его «unknown option».
        if err.contains("-json") {
            return Err("sqlite3 на сервере старше 3.33 и не умеет выводить JSON - нужен новее".into());
        }
        return Err(format!("SQLite: {}", err.lines().next().unwrap_or(err)));
    }
    parse_sqlite_json(&out, out.len() > MAX_BYTES)
}

/// Строка ответа `sqlite3 -json` с колонками в том порядке, в каком они в тексте.
///
/// Свой разбор, а не `serde_json::Value`: у нас `Value` хранит ключи объекта отсортированными,
/// и колонки `SELECT b, a` приходили бы как `a, b`. Включить сохранение порядка в `serde_json`
/// значило бы поменять его поведение во всём приложении ради одного места.
struct OrderedRow(Vec<(String, Value)>);

impl<'de> serde::Deserialize<'de> for OrderedRow {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = OrderedRow;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("строку ответа sqlite3")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<OrderedRow, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = a.next_entry::<String, Value>()? {
                    out.push((k, v));
                }
                Ok(OrderedRow(out))
            }
        }
        d.deserialize_map(V)
    }
}

fn sqlite_set(rows: Vec<OrderedRow>) -> ResultSet {
    let names: Vec<String> = rows
        .first()
        .map(|r| r.0.iter().map(|(k, _)| k.clone()).collect())
        .unwrap_or_default();
    let mut set = ResultSet::new(unique_names(names.iter().map(String::as_str)));
    for row in &rows {
        let mut obj = Map::new();
        let mut size = 0usize;
        for (i, name) in set.columns.iter().enumerate() {
            let cell = match row.0.get(i).map(|(_, v)| v) {
                None | Some(Value::Null) => Value::Null,
                Some(Value::String(s)) => {
                    size += s.len();
                    set.budget.cell(s)
                }
                Some(other) => {
                    let s = other.to_string();
                    size += s.len();
                    set.budget.cell(&s)
                }
            };
            obj.insert(name.clone(), cell);
        }
        if !set.budget.push(Value::Object(obj), size) {
            break;
        }
    }
    set
}

/// Разбирает вывод `sqlite3 -json`: по массиву на каждую выборку, выборки без строк
/// `sqlite3` не печатает вовсе.
///
/// Если вывод обрезан на сервере, последний массив оборван. Из него берём целые строки до
/// последней законченной: половина таблицы честнее, чем пустая таблица с ошибкой разбора.
fn parse_sqlite_json(out: &str, capped: bool) -> Result<Value, String> {
    let mut sets = Vec::new();
    let mut truncated = capped;
    let mut stream = serde_json::Deserializer::from_str(out).into_iter::<Vec<OrderedRow>>();
    loop {
        let start = stream.byte_offset();
        match stream.next() {
            None => break,
            Some(Ok(rows)) => sets.push(sqlite_set(rows)),
            Some(Err(e)) => {
                if !capped {
                    return Err(format!("SQLite вернул непонятный ответ: {e}"));
                }
                let rest = &out[start..];
                if let Some(cut) = rest.rfind("},\n{") {
                    if let Ok(rows) = serde_json::from_str::<Vec<OrderedRow>>(&format!("{}]", &rest[..cut + 1])) {
                        sets.push(sqlite_set(rows));
                    }
                }
                truncated = true;
                break;
            }
        }
    }
    let mut v = answer(sets);
    if truncated {
        v["truncated"] = json!(true);
    }
    Ok(v)
}

/// Запрос к MongoDB. Документы разной формы сводятся в одну таблицу: колонки - все поля
/// верхнего уровня в порядке первого появления.
async fn mongo_query(conn: &AsyncMutex<MongoConn>, text: &str) -> Result<Value, String> {
    // Разбор до замка: ошибка в тексте запроса не должна ждать чужого долгого запроса.
    let op = crate::mongo::parse(text)?;
    let out = {
        let mut g = conn.lock().await;
        g.run(op, MAX_ROWS, MAX_BYTES, SERVER_LIMIT).await?
    };
    Ok(answer(vec![mongo_set(out)]))
}

/// Колонок не больше этого. Коллекция, где у каждого документа свои поля, иначе дала бы
/// таблицу в тысячи столбцов, в которой ничего не разглядеть.
const MONGO_MAX_COLUMNS: usize = 300;

fn mongo_set(out: crate::mongo::Outcome) -> ResultSet {
    let mut names: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut too_wide = false;
    for d in &out.docs {
        for k in d.keys() {
            if seen.contains(k.as_str()) {
                continue;
            }
            if names.len() >= MONGO_MAX_COLUMNS {
                too_wide = true;
                break;
            }
            seen.insert(k.clone());
            names.push(k.clone());
        }
    }
    let mut set = ResultSet::new(names);
    set.affected = out.affected;
    for d in &out.docs {
        let mut obj = Map::new();
        let mut size = 0usize;
        for name in &set.columns {
            // Поля нет или оно null - в таблице одно и то же: пустая ячейка.
            let cell = match d.get(name).and_then(crate::mongo::cell) {
                Some(v) => {
                    size += v.len();
                    set.budget.cell(&v)
                }
                None => Value::Null,
            };
            obj.insert(name.clone(), cell);
        }
        if !set.budget.push(Value::Object(obj), size) {
            break;
        }
    }
    if out.cut || too_wide {
        set.budget.truncated = true;
    }
    set
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
    fn документы_mongodb_сводятся_в_таблицу() {
        let out = crate::mongo::Outcome {
            docs: vec![bson::doc! { "b": 1, "a": "x" }, bson::doc! { "c": bson::Bson::Null, "a": "" }],
            cut: false,
            affected: 0,
        };
        let v = answer(vec![mongo_set(out)]);
        assert_eq!(v["columns"], json!(["b", "a", "c"]), "поля в порядке первого появления");
        assert_eq!(v["rows"][0]["b"], "1");
        assert!(v["rows"][1]["b"].is_null(), "поля нет - ячейка пустая");
        assert_eq!(v["rows"][1]["a"], "", "пустая строка - не null");
    }

    #[test]
    fn ответ_sqlite3_разбирается_с_порядком_колонок() {
        // `Value` у нас сортирует ключи - порядок колонок обязан браться из самого текста.
        let out = "[{\"b\":\"x\",\"a\":1},\n{\"b\":\"\",\"a\":null}]\n[{\"n\":2}]\n";
        let v = parse_sqlite_json(out, false).expect("разбор");
        let sets = v["sets"].as_array().expect("наборы");
        assert_eq!(sets.len(), 2, "две выборки - два набора");
        assert_eq!(sets[0]["columns"], json!(["b", "a"]), "колонки в порядке запроса");
        assert_eq!(sets[0]["rows"][0]["a"], "1");
        assert_eq!(sets[0]["rows"][1]["b"], "", "пустая строка - не NULL");
        assert!(sets[0]["rows"][1]["a"].is_null(), "NULL остаётся NULL");
    }

    #[test]
    fn обрезанный_ответ_sqlite3_отдаёт_целые_строки() {
        // Вывод оборван на сервере посреди третьей строки.
        let out = "[{\"a\":1},\n{\"a\":2},\n{\"a\":3";
        let v = parse_sqlite_json(out, true).expect("разбор");
        assert_eq!(v["truncated"], true, "об обрезке надо сказать");
        assert_eq!(v["sets"][0]["rows"].as_array().expect("строки").len(), 2, "две целые строки: {v}");
    }

    #[test]
    fn непонятный_ответ_sqlite3_без_обрезки_это_ошибка() {
        assert!(parse_sqlite_json("[{\"a\":", false).is_err());
    }

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
