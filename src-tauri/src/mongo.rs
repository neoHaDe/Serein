//! MongoDB поверх **готового потока**.
//!
//! Почему свой протокол, как и у MySQL. Официальный драйвер `mongodb` открывает сокеты
//! сам: ему дают адрес, а не поток, и отдать ему канал внутри SSH-сессии нельзя. Остался бы
//! проброс порта на машину пользователя - тот самый открытый порт, от которого панель баз
//! и избавляет. Поэтому разговор с сервером ведётся здесь.
//!
//! Своё написано только то, чего никто не сделает за нас: упаковка сообщений `OP_MSG`,
//! рукопожатие, вход по SCRAM и чтение курсоров. Кодирование BSON берёт `bson` - крейт из
//! того же репозитория, что и официальный драйвер; хеши, HMAC и PBKDF2 - те же крейты, на
//! которых вход по SCRAM делает `tokio-postgres`. Криптографию мы не сочиняем.
//!
//! Пароль не уходит ни открытым текстом, ни хешем, который можно предъявить повторно: SCRAM
//! доказывает знание пароля, а сервер в ответ доказывает, что знает его сам. Подменённый
//! сервер на этом шаге и отсеивается.
//!
//! Язык запросов - подмножество mongosh: `db.коллекция.find(...)`, `show dbs`, `use база`
//! и документ команды как есть. JavaScript не исполняется: разбираются литералы и известные
//! вызовы, всё остальное - понятная ошибка, а не молчаливая догадка.

use base64::Engine as _;
use bson::raw::CString;
use bson::spec::BinarySubtype;
use bson::{doc, oid::ObjectId, Binary, Bson, DateTime, Decimal128, Document, Regex, Timestamp};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Код `OP_MSG` - единственного вида сообщений, на котором говорят серверы начиная с 3.6.
const OP_MSG: i32 = 2013;
/// Код ответа сервера на неизвестную команду.
const COMMAND_NOT_FOUND: i64 = 59;
/// Предел одного сообщения от сервера. Сам сервер шлёт не больше 48 МиБ; длина больше -
/// это рассинхрон или вовсе не MongoDB на том конце, и выделять под неё память нельзя.
const MAX_MESSAGE: usize = 64 * 1024 * 1024;
/// Сколько документов просим за раз при чтении курсора.
const BATCH: i32 = 500;
/// Глубина вложенности в разборе запроса. BSON сам не глубже сотни уровней, а без предела
/// строка из тысяч `[` уронила бы разбор переполнением стека.
const MAX_DEPTH: usize = 100;

/// Соединение с сервером. Живёт поверх любого потока - у нас это канал SSH.
pub struct Conn<S> {
    stream: S,
    request_id: i32,
    /// Текущая база: в неё идут запросы `db.…`. Меняется командой `use`.
    db: String,
}

/// Что вернул запрос: документы для таблицы и сколько документов он изменил.
#[derive(Debug)]
pub struct Outcome {
    pub docs: Vec<Document>,
    /// Показано не всё: курсор закрыт раньше конца, потому что кончилось место.
    pub cut: bool,
    pub affected: u64,
}

fn single(d: Document, affected: u64) -> Result<Outcome, String> {
    Ok(Outcome { docs: vec![d], cut: false, affected })
}

impl<S: AsyncRead + AsyncWrite + Unpin> Conn<S> {
    /// Приветствие и вход. Пустой пользователь - сервер без проверки доступа.
    pub async fn connect(
        stream: S,
        user: &str,
        password: &str,
        database: &str,
    ) -> Result<Self, String> {
        let db = match database.trim() {
            "" => "test".to_owned(),
            d => db_name(d)?,
        };
        let mut c = Conn { stream, request_id: 0, db };
        let user = user.trim();
        let (hello, reply) = c.handshake(user).await?;
        if user.is_empty() {
            return Ok(c);
        }

        // Пользователь может жить не в admin, а в самой базе - так заводят учётки приложений.
        // Где он есть, сервер говорит сам: у незнакомого пользователя список способов входа
        // пуст. Спрашиваем, а не пробуем входить наугад: каждая неудачная попытка пишется в
        // журнал сервера и может сработать на защиту от перебора.
        let mut source = "admin".to_owned();
        let mut mechs = mechanisms(&reply);
        if mechs.is_empty() && c.db != "admin" {
            let mut ask = Document::new();
            ask.insert(hello, 1);
            ask.insert("saslSupportedMechs", format!("{}.{user}", c.db));
            let r = c.command("admin", ask).await?;
            let m = mechanisms(&r);
            if !m.is_empty() {
                source = c.db.clone();
                mechs = m;
            }
        }
        let mech = if mechs.is_empty() || mechs.iter().any(|m| m == "SCRAM-SHA-256") {
            Mech::Sha256
        } else if mechs.iter().any(|m| m == "SCRAM-SHA-1") {
            Mech::Sha1
        } else {
            return Err(format!(
                "MongoDB предлагает этому пользователю только {} - поддерживается вход SCRAM-SHA-256 и SCRAM-SHA-1",
                mechs.join(", ")
            ));
        };
        c.auth(&source, mech, user, password).await?;
        Ok(c)
    }

    /// Текущая база соединения.
    pub fn database(&self) -> &str {
        &self.db
    }

    /// Приветствие. `hello` появился в 4.4.2 и 5.0; серверы старше понимают только
    /// `isMaster`, и на них повторяем тем же словом.
    async fn handshake(&mut self, user: &str) -> Result<(&'static str, Document), String> {
        for name in ["hello", "isMaster"] {
            let mut d = Document::new();
            d.insert(name, 1);
            if name == "hello" {
                // Имя приложения видно в журнале сервера и в `currentOp`: администратор
                // должен понимать, чьи это соединения.
                d.insert(
                    "client",
                    doc! {
                        "application": { "name": "Serein" },
                        "driver": { "name": "Serein", "version": env!("CARGO_PKG_VERSION") },
                        "os": { "type": std::env::consts::OS },
                    },
                );
            }
            if !user.is_empty() {
                d.insert("saslSupportedMechs", format!("admin.{user}"));
            }
            let (r, _) = self.roundtrip("admin", d).await?;
            if ok(&r) {
                return Ok((name, r));
            }
            if name != "hello" || number(&r, "code") != Some(COMMAND_NOT_FOUND) {
                return Err(server_error(&r));
            }
        }
        Err("MongoDB не ответил на приветствие".into())
    }

    /// Вход по SCRAM: два обмена и проверка подписи сервера.
    async fn auth(&mut self, source: &str, mech: Mech, user: &str, password: &str) -> Result<(), String> {
        let secret: Vec<u8> = match mech {
            // У SCRAM-SHA-1 в MongoDB паролем служит не сам пароль, а MD5 от
            // «пользователь:mongo:пароль» - так исторически хранит сервер.
            Mech::Sha1 => format!("{:x}", md5::compute(format!("{user}:mongo:{password}"))).into_bytes(),
            Mech::Sha256 => stringprep::saslprep(password)
                .map_err(|_| "Пароль содержит символы, недопустимые для SCRAM-SHA-256".to_owned())?
                .into_owned()
                .into_bytes(),
        };
        let nonce = {
            use rand::RngCore as _;
            let mut b = [0u8; 24];
            rand::thread_rng().fill_bytes(&mut b);
            B64.encode(b)
        };
        let first_bare = client_first_bare(user, &nonce);
        let start = doc! {
            "saslStart": 1,
            "mechanism": mech.name(),
            "payload": generic(format!("n,,{first_bare}").into_bytes()),
            "autoAuthorize": 1,
            "options": { "skipEmptyExchange": true },
        };
        let r = self.command(source, start).await?;
        let conversation = r.get("conversationId").cloned().unwrap_or(Bson::Int32(1));
        let (last, server_signature) = client_final(mech, &secret, &first_bare, &payload(&r)?, &nonce)?;
        let r = self
            .command(
                source,
                doc! {
                    "saslContinue": 1,
                    "conversationId": conversation.clone(),
                    "payload": generic(last.into_bytes()),
                },
            )
            .await?;
        verify_server_final(&payload(&r)?, &server_signature)?;
        // Старые серверы не знают `skipEmptyExchange` и ждут ещё один пустой обмен.
        let mut done = r.get_bool("done").unwrap_or(false);
        for _ in 0..3 {
            if done {
                return Ok(());
            }
            let r = self
                .command(
                    source,
                    doc! {
                        "saslContinue": 1,
                        "conversationId": conversation.clone(),
                        "payload": generic(Vec::new()),
                    },
                )
                .await?;
            done = r.get_bool("done").unwrap_or(false);
        }
        if done {
            Ok(())
        } else {
            Err("MongoDB не завершил вход".into())
        }
    }

    /// Отправляет команду и читает ответ, не заглядывая в `ok`. Вторым - длина ответа.
    async fn roundtrip(&mut self, db: &str, mut cmd: Document) -> Result<(Document, usize), String> {
        cmd.insert("$db", db);
        let mut body = Vec::new();
        cmd.to_writer(&mut body)
            .map_err(|e| format!("MongoDB: команда не упаковалась: {e}"))?;
        self.request_id = self.request_id.wrapping_add(1);
        self.stream.write_all(&frame(self.request_id, &body)).await.map_err(io_err)?;
        self.stream.flush().await.map_err(io_err)?;

        let mut head = [0u8; 16];
        self.stream.read_exact(&mut head).await.map_err(io_err)?;
        let (len, response_to, opcode) = header(&head);
        if !(21..=MAX_MESSAGE).contains(&len) {
            return Err(format!(
                "MongoDB: ответ длиной {len} Б - на этом адресе не MongoDB или поток разошёлся"
            ));
        }
        let mut rest = vec![0u8; len - 16];
        self.stream.read_exact(&mut rest).await.map_err(io_err)?;
        if opcode != OP_MSG {
            return Err(format!("MongoDB: неожиданный вид ответа {opcode}"));
        }
        if response_to != self.request_id {
            return Err("MongoDB: ответ пришёл не на тот запрос".into());
        }
        Ok((parse_msg(&rest)?, len))
    }

    async fn command(&mut self, db: &str, cmd: Document) -> Result<Document, String> {
        let (r, _) = self.roundtrip(db, cmd).await?;
        check(r)
    }

    async fn cursor(&mut self, db: &str, cmd: Document, rows: usize, bytes: usize) -> Result<Outcome, String> {
        let (r, size) = self.roundtrip(db, cmd).await?;
        let r = check(r)?;
        self.drain(db, r, size, rows, bytes).await
    }

    /// Дочитывает курсор, пока документы укладываются в пределы.
    ///
    /// Место кончилось, а курсор ещё открыт - закрываем его сразу: на сервере он жил бы до
    /// своего таймаута и держал память.
    async fn drain(
        &mut self,
        db: &str,
        mut reply: Document,
        mut bytes: usize,
        max_rows: usize,
        max_bytes: usize,
    ) -> Result<Outcome, String> {
        let mut docs = Vec::new();
        let mut batch_key = "firstBatch";
        loop {
            let Some(Bson::Document(mut cursor)) = reply.remove("cursor") else {
                return Err("MongoDB: в ответе нет курсора".into());
            };
            let id = number(&cursor, "id").unwrap_or(0);
            let ns = cursor.get_str("ns").unwrap_or("").to_owned();
            if let Some(Bson::Array(items)) = cursor.remove(batch_key) {
                docs.extend(items.into_iter().filter_map(|i| match i {
                    Bson::Document(d) => Some(d),
                    _ => None,
                }));
            }
            batch_key = "nextBatch";
            let over = docs.len() > max_rows;
            docs.truncate(max_rows);
            if id == 0 {
                return Ok(Outcome { docs, cut: over, affected: 0 });
            }
            // Имя коллекции для getMore - всё после первой точки: `база.коллекция`.
            let coll = ns.split_once('.').map(|(_, c)| c.to_owned()).unwrap_or_default();
            if docs.len() >= max_rows || bytes > max_bytes {
                let _ = self
                    .command(db, doc! { "killCursors": coll.as_str(), "cursors": [Bson::Int64(id)] })
                    .await;
                return Ok(Outcome { docs, cut: true, affected: 0 });
            }
            let (r, size) = self
                .roundtrip(
                    db,
                    doc! { "getMore": Bson::Int64(id), "collection": coll.as_str(), "batchSize": BATCH },
                )
                .await?;
            reply = check(r)?;
            bytes = bytes.saturating_add(size);
        }
    }

    /// Выполняет разобранный запрос.
    pub async fn run(&mut self, op: Op, max_rows: usize, max_bytes: usize) -> Result<Outcome, String> {
        let db = self.db.clone();
        match op {
            Op::Command { admin, cmd } => {
                let target = if admin { "admin".to_owned() } else { db };
                let (r, size) = self.roundtrip(&target, cmd).await?;
                let r = check(r)?;
                // Команда, отвечающая курсором (`find`, `aggregate`, `listIndexes` в виде
                // документа), показывается документами, а не одной строкой с курсором внутри.
                if matches!(r.get("cursor"), Some(Bson::Document(c)) if c.contains_key("firstBatch")) {
                    return self.drain(&target, r, size, max_rows, max_bytes).await;
                }
                single(clean(r), 0)
            }
            Op::Find { coll, spec, one } => {
                let mut cmd = doc! { "find": coll, "filter": spec.filter };
                if let Some(p) = spec.projection {
                    cmd.insert("projection", p);
                }
                if let Some(s) = spec.sort {
                    cmd.insert("sort", s);
                }
                if let Some(n) = spec.skip {
                    cmd.insert("skip", n);
                }
                if one {
                    cmd.insert("limit", 1i64);
                    cmd.insert("singleBatch", true);
                } else if let Some(n) = spec.limit.filter(|n| *n != 0) {
                    // Отрицательный предел в mongosh значит «одной пачкой и закрыть курсор».
                    cmd.insert("limit", n.abs());
                    if n < 0 {
                        cmd.insert("singleBatch", true);
                    }
                }
                cmd.insert("batchSize", BATCH);
                self.cursor(&db, cmd, max_rows, max_bytes).await
            }
            Op::Aggregate { coll, pipeline } => {
                let target = match coll {
                    Some(c) => Bson::String(c),
                    None => Bson::Int32(1),
                };
                let cmd = doc! { "aggregate": target, "pipeline": pipeline, "cursor": { "batchSize": BATCH } };
                self.cursor(&db, cmd, max_rows, max_bytes).await
            }
            Op::Count { coll, filter } => {
                // Как считает сам драйвер: `count` с фильтром на шардированных коллекциях
                // и после сбоев ошибается, а конвейер - нет.
                let cmd = doc! {
                    "aggregate": coll,
                    "pipeline": [ { "$match": filter }, { "$group": { "_id": 1, "n": { "$sum": 1 } } } ],
                    "cursor": {},
                };
                let out = self.cursor(&db, cmd, 1, max_bytes).await?;
                let n = out.docs.first().and_then(|d| number(d, "n")).unwrap_or(0);
                single(doc! { "количество": n }, 0)
            }
            Op::Estimated { coll } => {
                let r = self.command(&db, doc! { "count": coll }).await?;
                single(doc! { "количество": number(&r, "n").unwrap_or(0) }, 0)
            }
            Op::Distinct { coll, key, filter } => {
                let r = self.command(&db, doc! { "distinct": coll, "key": key, "query": filter }).await?;
                let values = match r.get("values") {
                    Some(Bson::Array(v)) => v.clone(),
                    _ => Vec::new(),
                };
                let cut = values.len() > max_rows;
                let docs = values.into_iter().take(max_rows).map(|v| doc! { "значение": v }).collect();
                Ok(Outcome { docs, cut, affected: 0 })
            }
            Op::Insert { coll, docs, many } => {
                let docs: Vec<Document> = docs.into_iter().map(with_id).collect();
                let first_id = docs.first().and_then(|d| d.get("_id")).cloned();
                let r = self
                    .command(&db, doc! { "insert": coll, "documents": docs, "ordered": true })
                    .await?;
                let n = number(&r, "n").unwrap_or(0);
                let mut d = doc! { "вставлено": n };
                if let (false, Some(id)) = (many, first_id) {
                    d.insert("_id", id);
                }
                single(d, n.max(0) as u64)
            }
            Op::Update { coll, filter, update, multi, upsert } => {
                let cmd = doc! {
                    "update": coll,
                    "updates": [ { "q": filter, "u": update, "multi": multi, "upsert": upsert } ],
                    "ordered": true,
                };
                let r = self.command(&db, cmd).await?;
                let matched = number(&r, "n").unwrap_or(0);
                let modified = number(&r, "nModified").unwrap_or(0);
                let mut d = doc! { "найдено": matched, "изменено": modified };
                let mut upserted = 0u64;
                if let Some(Bson::Array(list)) = r.get("upserted") {
                    upserted = list.len() as u64;
                    if let Some(Bson::Document(u)) = list.first() {
                        if let Some(id) = u.get("_id") {
                            d.insert("вставлен _id", id.clone());
                        }
                    }
                }
                single(d, modified.max(0) as u64 + upserted)
            }
            Op::Delete { coll, filter, multi } => {
                let limit = if multi { 0 } else { 1 };
                let cmd = doc! { "delete": coll, "deletes": [ { "q": filter, "limit": limit } ], "ordered": true };
                let r = self.command(&db, cmd).await?;
                let n = number(&r, "n").unwrap_or(0);
                single(doc! { "удалено": n }, n.max(0) as u64)
            }
            Op::CreateIndex { coll, keys, options } => {
                let mut index = doc! { "key": keys.clone(), "name": index_name(&keys) };
                for (k, v) in options {
                    index.insert(k, v);
                }
                let r = self.command(&db, doc! { "createIndexes": coll, "indexes": [index] }).await?;
                single(clean(r), 0)
            }
            Op::Indexes { coll } => {
                let cmd = doc! { "listIndexes": coll, "cursor": { "batchSize": BATCH } };
                self.cursor(&db, cmd, max_rows, max_bytes).await
            }
            Op::Drop { coll } => {
                let r = self.command(&db, doc! { "drop": coll }).await?;
                single(clean(r), 0)
            }
            Op::DropDatabase => {
                self.command(&db, doc! { "dropDatabase": 1 }).await?;
                single(doc! { "удалена база": db }, 0)
            }
            Op::ShowDbs => {
                let r = self.command("admin", doc! { "listDatabases": 1 }).await?;
                let docs = match r.get("databases") {
                    Some(Bson::Array(list)) => list
                        .iter()
                        .filter_map(|d| d.as_document().cloned())
                        .collect(),
                    _ => Vec::new(),
                };
                Ok(Outcome { docs, cut: false, affected: 0 })
            }
            Op::ShowCollections => {
                let cmd = doc! {
                    "listCollections": 1,
                    "nameOnly": true,
                    // Без этого пользователь с правами на одну коллекцию получает отказ,
                    // хотя спрашивает ровно о том, что ему доступно.
                    "authorizedCollections": true,
                    "cursor": { "batchSize": BATCH },
                };
                self.cursor(&db, cmd, max_rows, max_bytes).await
            }
            Op::Use(name) => {
                self.db = name.clone();
                single(doc! { "текущая база": name }, 0)
            }
            Op::Version => {
                let r = self.command("admin", doc! { "buildInfo": 1 }).await?;
                single(doc! { "версия": r.get_str("version").unwrap_or("").to_owned() }, 0)
            }
            Op::Stats => {
                let r = self.command(&db, doc! { "dbStats": 1 }).await?;
                single(clean(r), 0)
            }
        }
    }
}

/// Сообщение `OP_MSG`: заголовок, флаги и один раздел с телом команды.
fn frame(request_id: i32, body: &[u8]) -> Vec<u8> {
    let len = 16 + 4 + 1 + body.len();
    let mut msg = Vec::with_capacity(len);
    msg.extend_from_slice(&(len as i32).to_le_bytes());
    msg.extend_from_slice(&request_id.to_le_bytes());
    msg.extend_from_slice(&0i32.to_le_bytes());
    msg.extend_from_slice(&OP_MSG.to_le_bytes());
    msg.extend_from_slice(&0u32.to_le_bytes());
    msg.push(0);
    msg.extend_from_slice(body);
    msg
}

/// Длина, номер запроса, на который это ответ, и код операции.
fn header(h: &[u8; 16]) -> (usize, i32, i32) {
    let int = |i: usize| i32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
    (usize::try_from(int(0)).unwrap_or(0), int(8), int(12))
}

/// Тело ответа `OP_MSG` без заголовка: флаги и разделы.
fn parse_msg(rest: &[u8]) -> Result<Document, String> {
    let broken = || "MongoDB: ответ повреждён".to_owned();
    if rest.len() < 5 {
        return Err(broken());
    }
    let flags = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
    // Нулевой бит - в хвосте контрольная сумма. Сами мы её не просим, но сервер вправе.
    let end = if flags & 1 != 0 { rest.len().checked_sub(4).ok_or_else(broken)? } else { rest.len() };
    let mut pos = 4;
    let mut body = None;
    while pos < end {
        let kind = rest[pos];
        pos += 1;
        if pos + 4 > end {
            return Err(broken());
        }
        let size = i32::from_le_bytes([rest[pos], rest[pos + 1], rest[pos + 2], rest[pos + 3]]);
        let size = usize::try_from(size).map_err(|_| broken())?;
        if size < 5 || pos + size > end {
            return Err(broken());
        }
        match kind {
            0 => {
                body = Some(
                    Document::from_reader(&rest[pos..pos + size])
                        .map_err(|e| format!("MongoDB: ответ не разобрался: {e}"))?,
                )
            }
            // Последовательность документов: серверы в ответах её не шлют, но пропустить
            // её честнее, чем упасть.
            1 => {}
            other => return Err(format!("MongoDB: неизвестный раздел ответа {other}")),
        }
        pos += size;
    }
    body.ok_or_else(broken)
}

fn io_err(e: std::io::Error) -> String {
    format!("MongoDB: соединение оборвалось: {e}")
}

fn ok(d: &Document) -> bool {
    match d.get("ok") {
        Some(Bson::Double(f)) => *f == 1.0,
        Some(Bson::Int32(n)) => *n == 1,
        Some(Bson::Int64(n)) => *n == 1,
        Some(Bson::Boolean(b)) => *b,
        _ => false,
    }
}

/// Целое из поля, каким бы числовым типом его ни прислали: сервер не обязан держать один.
fn number(d: &Document, key: &str) -> Option<i64> {
    num_of(d.get(key)?)
}

fn num_of(v: &Bson) -> Option<i64> {
    match v {
        Bson::Int32(n) => Some(i64::from(*n)),
        Bson::Int64(n) => Some(*n),
        Bson::Double(f) if f.is_finite() && f.fract() == 0.0 => Some(*f as i64),
        _ => None,
    }
}

/// Текст ошибки сервера. Номер и имя кода - первыми: по ним ошибку и ищут.
fn server_error(d: &Document) -> String {
    let msg = d.get_str("errmsg").unwrap_or("ошибка без описания");
    match (number(d, "code"), d.get_str("codeName")) {
        (Some(code), Ok(name)) => format!("MongoDB {code} ({name}): {msg}"),
        (Some(code), Err(_)) => format!("MongoDB {code}: {msg}"),
        _ => format!("MongoDB: {msg}"),
    }
}

fn check(d: Document) -> Result<Document, String> {
    if !ok(&d) {
        return Err(server_error(&d));
    }
    // Ошибки записи приходят при `ok: 1`: команда выполнилась, а документы - нет. Показать
    // «вставлено 0» без причины значило бы спрятать её.
    if let Some(Bson::Array(errors)) = d.get("writeErrors") {
        if let Some(Bson::Document(first)) = errors.first() {
            let done = number(&d, "n").unwrap_or(0);
            return Err(format!("{} (выполнено до ошибки: {done})", server_error(first)));
        }
    }
    if let Some(Bson::Document(wce)) = d.get("writeConcernError") {
        return Err(format!("{} - запись не подтверждена", server_error(wce)));
    }
    Ok(d)
}

/// Ответ команды без служебных полей: `ok` и время кластера человеку ничего не говорят.
fn clean(d: Document) -> Document {
    d.into_iter()
        .filter(|(k, _)| k != "ok" && k != "operationTime" && !k.starts_with('$'))
        .collect()
}

fn mechanisms(reply: &Document) -> Vec<String> {
    match reply.get("saslSupportedMechs") {
        Some(Bson::Array(list)) => list.iter().filter_map(|m| m.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    }
}

fn generic(bytes: Vec<u8>) -> Binary {
    Binary { subtype: BinarySubtype::Generic, bytes }
}

fn payload(r: &Document) -> Result<String, String> {
    match r.get("payload") {
        Some(Bson::Binary(b)) => {
            String::from_utf8(b.bytes.clone()).map_err(|_| "MongoDB: непонятный ответ при входе".to_owned())
        }
        _ => Err("MongoDB: в ответе при входе нет данных".into()),
    }
}

/// `_id` первым полем, как делают драйверы: без него сервер создал бы его сам, но
/// показать человеку, что именно вставлено, было бы нечем.
fn with_id(d: Document) -> Document {
    if d.contains_key("_id") {
        return d;
    }
    let mut out = doc! { "_id": ObjectId::new() };
    out.extend(d);
    out
}

/// Имя индекса по ключам, как его строит сервер и mongosh: `a_1_b_-1`.
fn index_name(keys: &Document) -> String {
    keys.iter()
        .map(|(k, v)| {
            let v = match v {
                Bson::String(s) => s.clone(),
                other => shell_text(other),
            };
            format!("{k}_{v}")
        })
        .collect::<Vec<_>>()
        .join("_")
}

// ---------------------------------------------------------------------------------------
// SCRAM

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mech {
    Sha1,
    Sha256,
}

impl Mech {
    fn name(self) -> &'static str {
        match self {
            Mech::Sha1 => "SCRAM-SHA-1",
            Mech::Sha256 => "SCRAM-SHA-256",
        }
    }

    fn hi(self, password: &[u8], salt: &[u8], rounds: u32) -> Vec<u8> {
        match self {
            Mech::Sha1 => pbkdf2::pbkdf2_hmac_array::<sha1::Sha1, 20>(password, salt, rounds).to_vec(),
            Mech::Sha256 => pbkdf2::pbkdf2_hmac_array::<sha2::Sha256, 32>(password, salt, rounds).to_vec(),
        }
    }

    fn hmac(self, key: &[u8], data: &[u8]) -> Vec<u8> {
        use hmac::{Hmac, Mac};
        match self {
            Mech::Sha1 => {
                let mut m = <Hmac<sha1::Sha1> as Mac>::new_from_slice(key).expect("HMAC принимает ключ любой длины");
                m.update(data);
                m.finalize().into_bytes().to_vec()
            }
            Mech::Sha256 => {
                let mut m = <Hmac<sha2::Sha256> as Mac>::new_from_slice(key).expect("HMAC принимает ключ любой длины");
                m.update(data);
                m.finalize().into_bytes().to_vec()
            }
        }
    }

    fn h(self, data: &[u8]) -> Vec<u8> {
        use sha2::Digest as _;
        match self {
            Mech::Sha1 => sha1::Sha1::digest(data).to_vec(),
            Mech::Sha256 => sha2::Sha256::digest(data).to_vec(),
        }
    }
}

/// Первое сообщение клиента без префикса `n,,`. Запятая и знак равенства в имени служебные.
fn client_first_bare(user: &str, nonce: &str) -> String {
    format!("n={},r={nonce}", user.replace('=', "=3D").replace(',', "=2C"))
}

/// Второе сообщение клиента и подпись, которую обязан предъявить сервер.
fn client_final(
    mech: Mech,
    password: &[u8],
    first_bare: &str,
    server_first: &str,
    nonce: &str,
) -> Result<(String, Vec<u8>), String> {
    let bad = || "MongoDB: непонятный ответ сервера при входе".to_owned();
    let field = |name: &str| server_first.split(',').find_map(|p| p.strip_prefix(name));
    let r = field("r=").ok_or_else(bad)?;
    let salt = B64.decode(field("s=").ok_or_else(bad)?).map_err(|_| bad())?;
    let rounds: u32 = field("i=").ok_or_else(bad)?.parse().map_err(|_| bad())?;
    // Сервер обязан продолжить наше случайное число своим, иначе это чужой разговор.
    if !r.starts_with(nonce) || r.len() <= nonce.len() {
        return Err("MongoDB: сервер при входе ответил не на наш запрос".into());
    }
    // Меньше 4096 итераций спецификация драйверов запрещает: подбор пароля по перехваченному
    // обмену становится дешёвым, и соглашаться на такое нельзя.
    if rounds < 4096 {
        return Err(format!(
            "MongoDB: сервер просит всего {rounds} итераций хеширования пароля - это небезопасно, вход прерван"
        ));
    }
    let salted = mech.hi(password, &salt, rounds);
    let client_key = mech.hmac(&salted, b"Client Key");
    let stored_key = mech.h(&client_key);
    let without_proof = format!("c=biws,r={r}");
    let auth_message = format!("{first_bare},{server_first},{without_proof}");
    let signature = mech.hmac(&stored_key, auth_message.as_bytes());
    let proof: Vec<u8> = client_key.iter().zip(&signature).map(|(a, b)| a ^ b).collect();
    let server_key = mech.hmac(&salted, b"Server Key");
    let server_signature = mech.hmac(&server_key, auth_message.as_bytes());
    Ok((format!("{without_proof},p={}", B64.encode(proof)), server_signature))
}

fn verify_server_final(payload: &str, expected: &[u8]) -> Result<(), String> {
    if let Some(e) = payload.strip_prefix("e=") {
        return Err(format!("MongoDB: вход отклонён: {e}"));
    }
    let got = payload
        .split(',')
        .find_map(|p| p.strip_prefix("v="))
        .and_then(|v| B64.decode(v).ok())
        .ok_or("MongoDB: сервер не подтвердил вход подписью")?;
    // Без раннего выхода: время сравнения не должно выдавать, сколько байт совпало.
    let same = got.len() == expected.len()
        && got.iter().zip(expected).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
    if same {
        Ok(())
    } else {
        Err("MongoDB: подпись сервера не сошлась - на том конце не тот сервер, вход прерван".into())
    }
}

// ---------------------------------------------------------------------------------------
// Разбор запроса

/// Условия выборки `find` и её цепочка `.sort().limit()...`.
#[derive(Debug, PartialEq, Default)]
pub struct FindSpec {
    pub filter: Document,
    pub projection: Option<Document>,
    pub sort: Option<Document>,
    pub skip: Option<i64>,
    pub limit: Option<i64>,
}

/// Разобранный запрос.
#[derive(Debug, PartialEq)]
pub enum Op {
    /// Команда как есть: `db.runCommand({...})`, `db.adminCommand({...})` или документ.
    Command { admin: bool, cmd: Document },
    Find { coll: String, spec: FindSpec, one: bool },
    /// Без коллекции - конвейер уровня базы (`$currentOp`, `$listLocalSessions`).
    Aggregate { coll: Option<String>, pipeline: Vec<Bson> },
    Count { coll: String, filter: Document },
    Estimated { coll: String },
    Distinct { coll: String, key: String, filter: Document },
    Insert { coll: String, docs: Vec<Document>, many: bool },
    Update { coll: String, filter: Document, update: Bson, multi: bool, upsert: bool },
    Delete { coll: String, filter: Document, multi: bool },
    CreateIndex { coll: String, keys: Document, options: Document },
    Indexes { coll: String },
    Drop { coll: String },
    DropDatabase,
    ShowDbs,
    ShowCollections,
    Use(String),
    Version,
    Stats,
}

/// Разбирает запрос в духе mongosh.
pub fn parse(text: &str) -> Result<Op, String> {
    let t = text.trim().trim_end_matches(';').trim_end();
    if t.is_empty() {
        return Err("Пустой запрос".into());
    }
    let words: Vec<&str> = t.split_whitespace().collect();
    match words.as_slice() {
        ["show", "dbs" | "databases"] => return Ok(Op::ShowDbs),
        ["show", "collections" | "tables"] => return Ok(Op::ShowCollections),
        ["use", name] => return Ok(Op::Use(db_name(name)?)),
        _ => {}
    }
    let mut p = Parser { s: t, pos: 0, depth: 0 };
    let op = if t.starts_with('{') {
        Op::Command { admin: false, cmd: p.document()? }
    } else {
        p.shell()?
    };
    p.ws();
    if p.pos < p.s.len() {
        return Err(p.error("лишний текст после запроса - выполняется один запрос за раз"));
    }
    Ok(op)
}

fn db_name(name: &str) -> Result<String, String> {
    let bad = name.is_empty()
        || name.len() > 63
        || name.chars().any(|c| matches!(c, '/' | '\\' | '.' | ' ' | '"' | '$' | '\0'));
    if bad {
        Err(format!("Недопустимое имя базы: «{name}»"))
    } else {
        Ok(name.to_owned())
    }
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Пробелы и комментарии `//` до конца строки.
    fn ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.bump();
            } else if self.s[self.pos..].starts_with("//") {
                while let Some(c) = self.bump() {
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.ws();
        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(self.error(&format!("ожидался «{c}»")))
        }
    }

    /// Ошибка с местом. Номер символа, а не байта: в запросе с кириллицей байты сбивают.
    fn error(&self, what: &str) -> String {
        let col = self.s[..self.pos].chars().count() + 1;
        let near: String = self.s[self.pos..].chars().take(20).collect();
        if near.is_empty() {
            format!("{what} (в конце запроса)")
        } else {
            format!("{what} (символ {col}: «{near}»)")
        }
    }

    fn ident(&mut self) -> Option<String> {
        self.ws();
        let start = self.pos;
        match self.peek() {
            Some(c) if c.is_alphabetic() || c == '_' || c == '$' => {
                self.bump();
            }
            _ => return None,
        }
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' || c == '$' {
                self.bump();
            } else {
                break;
            }
        }
        Some(self.s[start..self.pos].to_owned())
    }

    /// `db.коллекция.метод(...)`, `db['коллекция'].метод(...)`, `db.метод(...)`.
    fn shell(&mut self) -> Result<Op, String> {
        if self.ident().as_deref() != Some("db") {
            self.pos = 0;
            return Err(self.error(
                "запрос начинается с db., show или use - либо это документ команды в фигурных скобках",
            ));
        }
        // Имя коллекции может содержать точки: `db.system.users.find()` - это коллекция
        // `system.users`. Метод - последнее имя перед скобкой.
        let mut coll: Option<String> = None;
        loop {
            if self.eat('[') {
                self.ws();
                let Some('"' | '\'') = self.peek() else {
                    return Err(self.error("в db[...] ожидалось имя коллекции в кавычках"));
                };
                let name = self.string()?;
                self.expect(']')?;
                coll = Some(join(coll, name));
                continue;
            }
            if !self.eat('.') {
                return Err(self.error("ожидался вызов вида db.коллекция.find(...)"));
            }
            let name = self.ident().ok_or_else(|| self.error("ожидалось имя после точки"))?;
            if !self.eat('(') {
                coll = Some(join(coll, name));
                continue;
            }
            let args = self.args()?;
            match coll.take() {
                None if name == "getCollection" => coll = Some(arg_str(&name, &args, 0)?),
                None => return db_method(&name, &args),
                Some(c) => return self.coll_method(c, &name, &args),
            }
        }
    }

    fn coll_method(&mut self, coll: String, name: &str, a: &[Bson]) -> Result<Op, String> {
        Ok(match name {
            "find" | "findOne" => {
                at_most(name, a, 2)?;
                let mut spec = FindSpec {
                    filter: arg_doc(name, a, 0)?,
                    projection: arg_opt_doc(name, a, 1)?,
                    ..FindSpec::default()
                };
                if name == "find" && self.chain(&mut spec)? {
                    return Ok(Op::Count { coll, filter: spec.filter });
                }
                Op::Find { coll, spec, one: name == "findOne" }
            }
            "aggregate" => {
                at_most(name, a, 2)?;
                Op::Aggregate { coll: Some(coll), pipeline: arg_array(name, a, 0)? }
            }
            "countDocuments" => Op::Count { coll, filter: arg_doc(name, a, 0)? },
            "estimatedDocumentCount" => Op::Estimated { coll },
            "distinct" => Op::Distinct { coll, key: arg_str(name, a, 0)?, filter: arg_doc(name, a, 1)? },
            "insertOne" => Op::Insert { coll, docs: vec![arg_req_doc(name, a, 0)?], many: false },
            "insertMany" => {
                let mut docs = Vec::new();
                for item in arg_array(name, a, 0)? {
                    match item {
                        Bson::Document(d) => docs.push(d),
                        _ => return Err("insertMany: в списке должны быть документы {...}".into()),
                    }
                }
                if docs.is_empty() {
                    return Err("insertMany: список документов пуст".into());
                }
                Op::Insert { coll, docs, many: true }
            }
            "updateOne" | "updateMany" => {
                let filter = arg_req_doc(name, a, 0)?;
                let update = match a.get(1) {
                    Some(Bson::Document(d)) if d.keys().next().is_some_and(|k| k.starts_with('$')) => {
                        Bson::Document(d.clone())
                    }
                    Some(Bson::Array(p)) => Bson::Array(p.clone()),
                    _ => {
                        return Err(format!(
                            "{name}: второй аргумент - операторы вида {{ $set: {{...}} }} или конвейер [...]; заменить документ целиком - replaceOne"
                        ))
                    }
                };
                Op::Update { coll, filter, update, multi: name == "updateMany", upsert: upsert_of(name, a, 2)? }
            }
            "replaceOne" => {
                let filter = arg_req_doc(name, a, 0)?;
                let doc = arg_req_doc(name, a, 1)?;
                if doc.keys().any(|k| k.starts_with('$')) {
                    return Err("replaceOne: в документе замены не бывает операторов $ - для этого updateOne".into());
                }
                Op::Update { coll, filter, update: Bson::Document(doc), multi: false, upsert: upsert_of(name, a, 2)? }
            }
            "deleteOne" | "deleteMany" => {
                Op::Delete { coll, filter: arg_req_doc(name, a, 0)?, multi: name == "deleteMany" }
            }
            "createIndex" => Op::CreateIndex { coll, keys: arg_req_doc(name, a, 0)?, options: arg_doc(name, a, 1)? },
            "getIndexes" => Op::Indexes { coll },
            "drop" => Op::Drop { coll },
            other => {
                return Err(format!(
                    "{other}() не поддерживается. Есть: find, findOne, aggregate, countDocuments, estimatedDocumentCount, distinct, insertOne, insertMany, updateOne, updateMany, replaceOne, deleteOne, deleteMany, createIndex, getIndexes, drop"
                ))
            }
        })
    }

    /// Цепочка после `find`. `true` - запрос закончился `.count()`.
    fn chain(&mut self, spec: &mut FindSpec) -> Result<bool, String> {
        while self.eat('.') {
            let name = self.ident().ok_or_else(|| self.error("ожидалось имя после точки"))?;
            self.expect('(')?;
            let a = self.args()?;
            match name.as_str() {
                "sort" => spec.sort = Some(arg_req_doc(&name, &a, 0)?),
                "projection" => spec.projection = Some(arg_req_doc(&name, &a, 0)?),
                "limit" => spec.limit = Some(arg_num(&name, &a, 0)?),
                "skip" => spec.skip = Some(arg_num(&name, &a, 0)?),
                "toArray" | "pretty" => {}
                "count" => return Ok(true),
                other => {
                    return Err(format!(
                        "{other}() после find не поддерживается. Есть: sort, limit, skip, projection, count, toArray"
                    ))
                }
            }
        }
        Ok(false)
    }

    fn args(&mut self) -> Result<Vec<Bson>, String> {
        let mut out = Vec::new();
        if self.eat(')') {
            return Ok(out);
        }
        loop {
            out.push(self.value()?);
            if self.eat(',') {
                if self.eat(')') {
                    return Ok(out);
                }
                continue;
            }
            self.expect(')')?;
            return Ok(out);
        }
    }

    fn value(&mut self) -> Result<Bson, String> {
        self.ws();
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("слишком глубокая вложенность"));
        }
        let v = self.value_inner();
        self.depth -= 1;
        v
    }

    fn value_inner(&mut self) -> Result<Bson, String> {
        match self.peek() {
            None => Err(self.error("ожидалось значение")),
            Some('{') => self.document().map(Bson::Document),
            Some('[') => self.array().map(Bson::Array),
            Some('"' | '\'') => self.string().map(Bson::String),
            Some('/') => self.regex(),
            Some(c) if c.is_ascii_digit() || matches!(c, '-' | '+' | '.') => self.number(),
            Some(_) => {
                let start = self.pos;
                match self.ident() {
                    Some(word) => self.word(word, start),
                    None => Err(self.error("непонятное значение")),
                }
            }
        }
    }

    fn document(&mut self) -> Result<Document, String> {
        self.expect('{')?;
        let mut d = Document::new();
        loop {
            if self.eat('}') {
                return Ok(d);
            }
            self.ws();
            let key = match self.peek() {
                Some('"' | '\'') => self.string()?,
                _ => self.ident().ok_or_else(|| self.error("ожидалось имя поля"))?,
            };
            self.expect(':')?;
            let v = self.value()?;
            d.insert(key, v);
            if !self.eat(',') {
                self.expect('}')?;
                return Ok(d);
            }
        }
    }

    fn array(&mut self) -> Result<Vec<Bson>, String> {
        self.expect('[')?;
        let mut out = Vec::new();
        loop {
            if self.eat(']') {
                return Ok(out);
            }
            out.push(self.value()?);
            if !self.eat(',') {
                self.expect(']')?;
                return Ok(out);
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        let quote = self.bump().unwrap_or('"');
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.error("строка не закрыта")),
                Some(c) if c == quote => return Ok(out),
                Some('\\') => match self.bump() {
                    None => return Err(self.error("строка не закрыта")),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('b') => out.push('\u{8}'),
                    Some('f') => out.push('\u{c}'),
                    Some('0') => out.push('\0'),
                    Some('u') => out.push(self.unicode()?),
                    Some('\n') => {}
                    Some(c) => out.push(c),
                },
                Some(c) => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut v = 0;
        for _ in 0..4 {
            let d = self.bump().and_then(|c| c.to_digit(16));
            v = v * 16 + d.ok_or_else(|| self.error("после \\u ожидались четыре шестнадцатеричные цифры"))?;
        }
        Ok(v)
    }

    /// `\uXXXX`, в том числе суррогатной парой - так в JSON пишутся символы вне BMP.
    fn unicode(&mut self) -> Result<char, String> {
        let hi = self.hex4()?;
        if (0xD800..0xDC00).contains(&hi) && self.s[self.pos..].starts_with("\\u") {
            let save = self.pos;
            self.pos += 2;
            let lo = self.hex4()?;
            if (0xDC00..0xE000).contains(&lo) {
                let code = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                return Ok(char::from_u32(code).unwrap_or('\u{FFFD}'));
            }
            self.pos = save;
        }
        Ok(char::from_u32(hi).unwrap_or('\u{FFFD}'))
    }

    fn regex(&mut self) -> Result<Bson, String> {
        self.bump();
        let mut pattern = String::new();
        let mut class = false;
        loop {
            match self.bump() {
                None | Some('\n') => return Err(self.error("регулярное выражение не закрыто")),
                Some('\\') => {
                    pattern.push('\\');
                    if let Some(c) = self.bump() {
                        pattern.push(c);
                    }
                }
                Some('/') if !class => break,
                Some(c) => {
                    match c {
                        '[' => class = true,
                        ']' => class = false,
                        _ => {}
                    }
                    pattern.push(c);
                }
            }
        }
        let mut flags: Vec<char> = Vec::new();
        while let Some(c) = self.peek().filter(char::is_ascii_alphabetic) {
            if !"imsxlu".contains(c) {
                return Err(self.error(&format!("флаг регулярного выражения «{c}» MongoDB не поддерживает")));
            }
            flags.push(c);
            self.bump();
        }
        // Сервер требует флаги по алфавиту.
        flags.sort_unstable();
        flags.dedup();
        regex_bson(pattern, flags.into_iter().collect())
    }

    fn number(&mut self) -> Result<Bson, String> {
        let start = self.pos;
        if matches!(self.peek(), Some('-' | '+')) {
            self.bump();
        }
        if self.s[self.pos..].starts_with("Infinity") {
            self.pos += "Infinity".len();
            let negative = self.s[start..].starts_with('-');
            return Ok(Bson::Double(if negative { f64::NEG_INFINITY } else { f64::INFINITY }));
        }
        let mut float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.bump();
            } else if matches!(c, '.' | 'e' | 'E') {
                float = true;
                self.bump();
                if c != '.' && matches!(self.peek(), Some('-' | '+')) {
                    self.bump();
                }
            } else {
                break;
            }
        }
        let text = self.s[start..self.pos].to_owned();
        // Целое - целым, как пишет его mongosh: 32 бита, если влезает, иначе 64.
        if !float {
            if let Ok(n) = text.parse::<i64>() {
                return Ok(match i32::try_from(n) {
                    Ok(small) => Bson::Int32(small),
                    Err(_) => Bson::Int64(n),
                });
            }
        }
        match text.parse::<f64>() {
            Ok(f) => Ok(Bson::Double(f)),
            Err(_) => {
                self.pos = start;
                Err(self.error("непонятное число"))
            }
        }
    }

    /// Слово: `true`, `null`, `new Date(...)`, `ObjectId('...')` и другие обёртки mongosh.
    fn word(&mut self, word: String, start: usize) -> Result<Bson, String> {
        match word.as_str() {
            "true" => return Ok(Bson::Boolean(true)),
            "false" => return Ok(Bson::Boolean(false)),
            // `undefined` сервер давно не принимает, и mongosh сам пишет вместо него null.
            "null" | "undefined" => return Ok(Bson::Null),
            "NaN" => return Ok(Bson::Double(f64::NAN)),
            "Infinity" => return Ok(Bson::Double(f64::INFINITY)),
            "new" => {
                let at = self.pos;
                let next = self.ident().ok_or_else(|| self.error("после new ожидалось имя"))?;
                return self.word(next, at);
            }
            _ => {}
        }
        if !self.eat('(') {
            self.pos = start;
            return Err(self.error(&format!("неизвестное слово «{word}» - строки пишутся в кавычках")));
        }
        let args = self.args()?;
        let bad = |what: &str| format!("{word}(): {what}");
        match (word.as_str(), args.as_slice()) {
            ("ObjectId", []) => Ok(Bson::ObjectId(ObjectId::new())),
            ("ObjectId", [Bson::String(s)]) => ObjectId::parse_str(s)
                .map(Bson::ObjectId)
                .map_err(|_| bad("нужны 24 шестнадцатеричные цифры")),
            ("ISODate" | "Date", []) => Ok(Bson::DateTime(DateTime::now())),
            ("ISODate" | "Date", [Bson::String(s)]) => parse_date(s)
                .map(Bson::DateTime)
                .ok_or_else(|| bad("ожидалась дата вида 2026-09-13T10:20:30Z")),
            ("ISODate" | "Date", [n]) if num_of(n).is_some() => {
                Ok(Bson::DateTime(DateTime::from_millis(num_of(n).unwrap_or(0))))
            }
            ("NumberLong" | "Long", a) => int_arg(a).map(Bson::Int64).ok_or_else(|| bad("ожидалось целое число")),
            ("NumberInt" | "Int32", a) => int_arg(a)
                .and_then(|n| i32::try_from(n).ok())
                .map(Bson::Int32)
                .ok_or_else(|| bad("ожидалось целое число в пределах 32 бит")),
            ("NumberDecimal" | "Decimal128", [Bson::String(s)]) => s
                .parse::<Decimal128>()
                .map(Bson::Decimal128)
                .map_err(|_| bad("непонятное десятичное число")),
            ("UUID", [Bson::String(s)]) => bson::Uuid::parse_str(s)
                .map(|u| Bson::Binary(Binary::from_uuid(u)))
                .map_err(|_| bad("непонятный UUID")),
            ("Timestamp", [t, i]) => timestamp(num_of(t), num_of(i)).ok_or_else(|| bad("ожидались два целых числа")),
            ("Timestamp", [Bson::Document(d)]) => {
                timestamp(number(d, "t"), number(d, "i")).ok_or_else(|| bad("ожидалось {{ t: ..., i: ... }}"))
            }
            ("BinData", [sub, Bson::String(b64)]) => {
                let sub = num_of(sub).and_then(|n| u8::try_from(n).ok()).ok_or_else(|| bad("подтип - число от 0 до 255"))?;
                let bytes = B64.decode(b64).map_err(|_| bad("данные - строка base64"))?;
                Ok(Bson::Binary(Binary { subtype: BinarySubtype::from(sub), bytes }))
            }
            ("MinKey", []) => Ok(Bson::MinKey),
            ("MaxKey", []) => Ok(Bson::MaxKey),
            ("ObjectId" | "ISODate" | "Date" | "NumberDecimal" | "Decimal128" | "UUID" | "Timestamp" | "BinData" | "MinKey" | "MaxKey", _) => {
                Err(bad("неверные аргументы"))
            }
            _ => Err(format!("{word}() в запросе не поддерживается - JavaScript здесь не выполняется")),
        }
    }
}

fn join(coll: Option<String>, name: String) -> String {
    match coll {
        Some(c) => format!("{c}.{name}"),
        None => name,
    }
}

fn db_method(name: &str, a: &[Bson]) -> Result<Op, String> {
    let command = |admin: bool| -> Result<Op, String> {
        at_most(name, a, 1)?;
        let cmd = match a.first() {
            Some(Bson::Document(d)) if !d.is_empty() => d.clone(),
            // `db.runCommand('ping')` - сокращение mongosh для `{ ping: 1 }`.
            Some(Bson::String(s)) => doc! { s.as_str(): 1 },
            _ => return Err(format!("{name}: ожидался документ команды, например {{ ping: 1 }}")),
        };
        Ok(Op::Command { admin, cmd })
    };
    match name {
        "runCommand" => command(false),
        "adminCommand" => command(true),
        "aggregate" => Ok(Op::Aggregate { coll: None, pipeline: arg_array(name, a, 0)? }),
        "getCollectionNames" => Ok(Op::ShowCollections),
        "stats" => Ok(Op::Stats),
        "version" => Ok(Op::Version),
        "dropDatabase" => Ok(Op::DropDatabase),
        other => Err(format!(
            "db.{other}() не поддерживается. Есть: runCommand, adminCommand, aggregate, stats, version, getCollectionNames, getCollection, dropDatabase"
        )),
    }
}

fn at_most(method: &str, a: &[Bson], n: usize) -> Result<(), String> {
    if a.len() > n {
        Err(format!("{method}: слишком много аргументов"))
    } else {
        Ok(())
    }
}

/// Необязательный документ: нет аргумента - пустой документ.
fn arg_doc(method: &str, a: &[Bson], i: usize) -> Result<Document, String> {
    Ok(arg_opt_doc(method, a, i)?.unwrap_or_default())
}

fn arg_opt_doc(method: &str, a: &[Bson], i: usize) -> Result<Option<Document>, String> {
    match a.get(i) {
        None | Some(Bson::Null) => Ok(None),
        Some(Bson::Document(d)) => Ok(Some(d.clone())),
        Some(_) => Err(format!("{method}: аргумент {} должен быть документом {{...}}", i + 1)),
    }
}

/// Обязательный документ. Фильтр у изменяющих команд обязателен намеренно, как и в
/// mongosh: `deleteMany()` без условия - это не опечатка, которую стоит угадывать.
fn arg_req_doc(method: &str, a: &[Bson], i: usize) -> Result<Document, String> {
    arg_opt_doc(method, a, i)?.ok_or_else(|| format!("{method}: аргумент {} - документ {{...}} - обязателен", i + 1))
}

fn arg_array(method: &str, a: &[Bson], i: usize) -> Result<Vec<Bson>, String> {
    match a.get(i) {
        Some(Bson::Array(v)) => Ok(v.clone()),
        _ => Err(format!("{method}: аргумент {} должен быть списком [...]", i + 1)),
    }
}

fn arg_num(method: &str, a: &[Bson], i: usize) -> Result<i64, String> {
    a.get(i).and_then(num_of).ok_or_else(|| format!("{method}: ожидалось целое число"))
}

fn arg_str(method: &str, a: &[Bson], i: usize) -> Result<String, String> {
    match a.get(i) {
        Some(Bson::String(s)) if !s.is_empty() => Ok(s.clone()),
        _ => Err(format!("{method}: аргумент {} должен быть строкой", i + 1)),
    }
}

fn upsert_of(method: &str, a: &[Bson], i: usize) -> Result<bool, String> {
    Ok(arg_doc(method, a, i)?.get_bool("upsert").unwrap_or(false))
}

fn int_arg(a: &[Bson]) -> Option<i64> {
    match a {
        [Bson::String(s)] => s.trim().parse().ok(),
        [n] => num_of(n),
        _ => None,
    }
}

fn timestamp(t: Option<i64>, i: Option<i64>) -> Option<Bson> {
    Some(Bson::Timestamp(Timestamp {
        time: u32::try_from(t?).ok()?,
        increment: u32::try_from(i?).ok()?,
    }))
}

fn regex_bson(pattern: String, options: String) -> Result<Bson, String> {
    let nul = || "в регулярном выражении не бывает нулевого символа".to_owned();
    Ok(Bson::RegularExpression(Regex {
        pattern: CString::try_from(pattern).map_err(|_| nul())?,
        options: CString::try_from(options).map_err(|_| nul())?,
    }))
}

/// Дата, как её принимает ISODate: с зоной, без зоны (это UTC) и просто день.
fn parse_date(s: &str) -> Option<DateTime> {
    let s = s.trim();
    DateTime::parse_rfc3339_str(s)
        .or_else(|_| DateTime::parse_rfc3339_str(format!("{s}Z")))
        .or_else(|_| DateTime::parse_rfc3339_str(format!("{s}T00:00:00Z")))
        .ok()
}

// ---------------------------------------------------------------------------------------
// Показ значений

/// Значение ячейки таблицы. `None` - это null.
///
/// Строка, дата и десятичное число верхнего уровня - как есть. Остальное - в записи
/// mongosh, чтобы значение можно было скопировать обратно в запрос: `ObjectId('…')`
/// отличается от строки с теми же цифрами, и в фильтре это разные вещи.
pub fn cell(v: &Bson) -> Option<String> {
    match v {
        Bson::Null | Bson::Undefined => None,
        Bson::String(s) | Bson::Symbol(s) => Some(s.clone()),
        Bson::DateTime(d) => Some(date_text(*d)),
        Bson::Decimal128(d) => Some(d.to_string()),
        other => Some(shell_text(other)),
    }
}

fn shell_text(v: &Bson) -> String {
    let mut out = String::new();
    shell(v, &mut out);
    out
}

fn shell(v: &Bson, out: &mut String) {
    use std::fmt::Write as _;
    match v {
        Bson::Double(f) => out.push_str(&double_text(*f)),
        Bson::String(s) | Bson::Symbol(s) => quote(s, out),
        Bson::Array(items) if items.is_empty() => out.push_str("[]"),
        Bson::Array(items) => {
            out.push_str("[ ");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                shell(item, out);
            }
            out.push_str(" ]");
        }
        Bson::Document(d) if d.is_empty() => out.push_str("{}"),
        Bson::Document(d) => {
            out.push_str("{ ");
            for (i, (k, v)) in d.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                if is_ident(k) {
                    out.push_str(k);
                } else {
                    quote(k, out);
                }
                out.push_str(": ");
                shell(v, out);
            }
            out.push_str(" }");
        }
        Bson::Boolean(b) => {
            let _ = write!(out, "{b}");
        }
        Bson::Null => out.push_str("null"),
        Bson::Undefined => out.push_str("undefined"),
        Bson::RegularExpression(r) => {
            let _ = write!(out, "{r}");
        }
        Bson::JavaScriptCode(code) => out.push_str(code),
        Bson::JavaScriptCodeWithScope(c) => out.push_str(&c.code),
        Bson::Int32(n) => {
            let _ = write!(out, "{n}");
        }
        Bson::Int64(n) => {
            let _ = write!(out, "{n}");
        }
        Bson::Timestamp(t) => {
            let _ = write!(out, "Timestamp({{ t: {}, i: {} }})", t.time, t.increment);
        }
        Bson::Binary(b) if b.subtype == BinarySubtype::Uuid && b.bytes.len() == 16 => {
            let h = hex::encode(&b.bytes);
            let _ = write!(out, "UUID('{}-{}-{}-{}-{}')", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]);
        }
        Bson::Binary(b) => {
            let _ = write!(out, "BinData({}, '{}')", u8::from(b.subtype), B64.encode(&b.bytes));
        }
        Bson::ObjectId(o) => {
            let _ = write!(out, "ObjectId('{}')", o.to_hex());
        }
        Bson::DateTime(d) => {
            let _ = write!(out, "ISODate('{}')", date_text(*d));
        }
        Bson::Decimal128(d) => {
            let _ = write!(out, "Decimal128('{d}')");
        }
        Bson::MaxKey => out.push_str("MaxKey()"),
        Bson::MinKey => out.push_str("MinKey()"),
        Bson::DbPointer(_) => out.push_str("DBPointer()"),
    }
}

fn double_text(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else {
        format!("{f}")
    }
}

fn date_text(d: DateTime) -> String {
    d.try_to_rfc3339_string().unwrap_or_else(|_| format!("Date({})", d.timestamp_millis()))
}

fn is_ident(k: &str) -> bool {
    let mut chars = k.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

fn quote(s: &str, out: &mut String) {
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('\'');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn литералы_mongosh_разбираются_с_порядком_полей() {
        let Op::Command { cmd, .. } = parse("{ find: 'users', filter: { \"age\": { $gt: 30 } }, sort: { b: 1, a: -1, }, }").expect("разбор") else {
            panic!("ожидалась команда");
        };
        assert_eq!(cmd.keys().collect::<Vec<_>>(), vec!["find", "filter", "sort"], "имя команды обязано идти первым");
        let sort = cmd.get_document("sort").expect("sort");
        assert_eq!(sort.keys().collect::<Vec<_>>(), vec!["b", "a"], "порядок сортировки - порядок полей");
        assert_eq!(sort.get("a"), Some(&Bson::Int32(-1)));
        assert_eq!(cmd.get_document("filter").unwrap().get_document("age").unwrap().get("$gt"), Some(&Bson::Int32(30)));
    }

    #[test]
    fn обёртки_mongosh_становятся_типами_bson() {
        let Op::Command { cmd, .. } = parse(
            "{ a: ObjectId('650000000000000000000001'), b: ISODate('2026-09-13T10:20:30Z'), c: NumberLong('9000000000'), \
             d: 3000000000, e: 1.5, f: /ab\\/c/im, g: NumberDecimal('12.50'), h: new Date('2026-09-13'), i: 'стро\\'ка', j: null, k: [true, false], \
             l: UUID('01234567-89ab-cdef-0123-456789abcdef') }",
        )
        .expect("разбор") else {
            panic!("ожидалась команда");
        };
        assert!(matches!(cmd.get("a"), Some(Bson::ObjectId(_))));
        assert!(matches!(cmd.get("b"), Some(Bson::DateTime(_))));
        assert_eq!(cmd.get("c"), Some(&Bson::Int64(9_000_000_000)));
        assert_eq!(cmd.get("d"), Some(&Bson::Int64(3_000_000_000)), "за пределами 32 бит - 64");
        assert_eq!(cmd.get("e"), Some(&Bson::Double(1.5)));
        assert_eq!(cell(cmd.get("f").unwrap()).unwrap(), "/ab\\/c/im");
        assert_eq!(cell(cmd.get("g").unwrap()).unwrap(), "12.50");
        assert_eq!(cell(cmd.get("h").unwrap()).unwrap(), "2026-09-13T00:00:00Z", "день без времени - полночь UTC");
        assert_eq!(cmd.get_str("i").unwrap(), "стро'ка");
        assert_eq!(cmd.get("j"), Some(&Bson::Null));
        assert_eq!(cell(cmd.get("l").unwrap()).unwrap(), "UUID('01234567-89ab-cdef-0123-456789abcdef')");
    }

    #[test]
    fn вызовы_коллекций_и_цепочки() {
        let op = parse("db.system.users.find({ user: 'probe' }, { _id: 0 }).sort({ user: 1 }).skip(5).limit(20);").expect("разбор");
        let Op::Find { coll, spec, one } = op else { panic!("ожидался find") };
        assert_eq!(coll, "system.users", "точки в имени коллекции - часть имени");
        assert!(!one);
        assert_eq!(spec.filter, doc! { "user": "probe" });
        assert_eq!(spec.projection, Some(doc! { "_id": 0 }));
        assert_eq!(spec.sort, Some(doc! { "user": 1 }));
        assert_eq!((spec.skip, spec.limit), (Some(5), Some(20)));

        assert_eq!(
            parse("db.getCollection('с пробелом').find({ a: 1 }).count()").unwrap(),
            Op::Count { coll: "с пробелом".into(), filter: doc! { "a": 1 } }
        );
        assert_eq!(parse("db['x-y'].getIndexes()").unwrap(), Op::Indexes { coll: "x-y".into() });
        assert_eq!(parse("db.заказы.drop()").unwrap(), Op::Drop { coll: "заказы".into() });
        assert_eq!(parse("show dbs").unwrap(), Op::ShowDbs);
        assert_eq!(parse("use app;").unwrap(), Op::Use("app".into()));
        assert_eq!(parse("db.runCommand('ping')").unwrap(), Op::Command { admin: false, cmd: doc! { "ping": 1 } });
    }

    #[test]
    fn опасные_и_неполные_запросы_отвергаются_словами() {
        assert!(parse("db.t.deleteMany()").unwrap_err().contains("обязателен"), "удаление без фильтра не угадываем");
        assert!(parse("db.t.updateOne({ a: 1 }, { b: 2 })").unwrap_err().contains("replaceOne"));
        assert!(parse("db.t.replaceOne({ a: 1 }, { $set: { b: 2 } })").unwrap_err().contains("updateOne"));
        assert!(parse("db.t.find({ a: somevar })").unwrap_err().contains("кавычках"));
        assert!(parse("db.t.find({}); db.t.drop()").unwrap_err().contains("один запрос"));
        assert!(parse("db.t.mapReduce()").unwrap_err().contains("не поддерживается"));
        assert!(parse("use a.b").is_err());
        assert!(parse(&"[".repeat(10_000)).is_err(), "глубина ограничена, стек цел");
    }

    #[test]
    fn вложенные_значения_показываются_записью_mongosh() {
        let v = Bson::Document(doc! {
            "id": ObjectId::parse_str("650000000000000000000001").unwrap(),
            "теги": ["a", 1, 2.5],
            "с пробелом": { "x": Bson::Null },
            "пусто": [],
        });
        assert_eq!(
            cell(&v).unwrap(),
            "{ id: ObjectId('650000000000000000000001'), теги: [ 'a', 1, 2.5 ], 'с пробелом': { x: null }, пусто: [] }"
        );
        assert_eq!(cell(&Bson::Null), None, "null - это null, а не текст");
        assert_eq!(cell(&Bson::String(String::new())).unwrap(), "", "пустая строка - не null");
        assert_eq!(index_name(&doc! { "a": 1, "b": -1, "t": "text" }), "a_1_b_-1_t_text");
    }

    #[test]
    fn scram_sha_256_по_rfc_7677() {
        let nonce = "rOprNGfwEbeRWgbNEkqO";
        let bare = client_first_bare("user", nonce);
        assert_eq!(bare, "n=user,r=rOprNGfwEbeRWgbNEkqO");
        let server_first = "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096";
        let (last, sig) = client_final(Mech::Sha256, b"pencil", &bare, server_first, nonce).expect("шаг");
        assert_eq!(
            last,
            "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );
        verify_server_final("v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=", &sig).expect("подпись сервера");
        assert!(verify_server_final("v=AAAA", &sig).is_err(), "чужая подпись - отказ");
    }

    #[test]
    fn scram_sha_1_по_rfc_5802() {
        let nonce = "fyko+d2lbbFgONRv9qkxdawL";
        let bare = client_first_bare("user", nonce);
        let server_first = "r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,s=QSXCR+Q6sek8bf92,i=4096";
        let (last, sig) = client_final(Mech::Sha1, b"pencil", &bare, server_first, nonce).expect("шаг");
        assert_eq!(last, "c=biws,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,p=v0X8v3Bz2T0CJGbJQyF0X+HI4Ts=");
        verify_server_final("v=rmF9pqV8S7suAoZWja4dJRkFsKQ=", &sig).expect("подпись сервера");
    }

    #[test]
    fn scram_не_соглашается_на_чужое_число_и_слабое_хеширование() {
        let bare = client_first_bare("user", "abc");
        assert!(client_final(Mech::Sha256, b"p", &bare, "r=xyz123,s=AAAA,i=4096", "abc").is_err());
        let weak = client_final(Mech::Sha256, b"p", &bare, "r=abc123,s=AAAA,i=1", "abc").unwrap_err();
        assert!(weak.contains("итераций"), "{weak}");
        assert_eq!(client_first_bare("a=b,c", "n"), "n=a=3Db=2Cc,r=n");
    }

    #[test]
    fn ответ_op_msg_разбирается_и_ошибка_несёт_код() {
        let reply = doc! { "ok": 0.0, "errmsg": "no such command: 'нет'", "code": 59, "codeName": "CommandNotFound" };
        let mut body = Vec::new();
        reply.to_writer(&mut body).unwrap();
        let msg = frame(7, &body);
        let (len, _, opcode) = header(msg[..16].try_into().unwrap());
        assert_eq!((len, opcode), (msg.len(), OP_MSG));
        let parsed = parse_msg(&msg[16..]).expect("разбор");
        assert_eq!(check(parsed).unwrap_err(), "MongoDB 59 (CommandNotFound): no such command: 'нет'");

        let partial = doc! { "ok": 1, "n": 1, "writeErrors": [ { "index": 1, "code": 11000, "errmsg": "E11000 duplicate key" } ] };
        assert_eq!(check(partial).unwrap_err(), "MongoDB 11000: E11000 duplicate key (выполнено до ошибки: 1)");
        assert!(parse_msg(&[0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0x7F]).is_err(), "длина раздела за пределами ответа");
    }
}
