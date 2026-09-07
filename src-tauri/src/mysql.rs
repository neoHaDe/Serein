//! MySQL и MariaDB поверх **готового потока**.
//!
//! Почему свой протокол, а не клиент с полки. Ни `mysql_async`, ни `sqlx` не умеют
//! принимать уже открытое соединение: сокет они создают сами (`connect_tcp` там
//! `pub(crate)`), и отдать им канал внутри SSH-сессии невозможно. Для PostgreSQL и Redis
//! такая точка входа нашлась — поэтому они и оказались первыми, — а здесь пришлось бы
//! возвращаться к пробросу порта на машину пользователя. Открытый порт ради третьей
//! по счёту базы — плохой размен, поэтому протокол разбирается здесь.
//!
//! Своё написано только то, чего никто не сделает за нас: нарезка на пакеты и порядок
//! шагов. Разбор структур, скрэмблы и шифрование пароля берёт `mysql_common` — там же,
//! откуда их берёт `mysql_async`, так что криптографию мы не сочиняем.
//!
//! Пароль в открытом виде не уходит никогда. `caching_sha2_password` (умолчание MySQL 8)
//! при первом входе просит полную аутентификацию; мы идём длинным путём — запрашиваем
//! у сервера открытый ключ и шифруем пароль RSA, — хотя канал и так внутри SSH и можно
//! было бы объявить транспорт доверенным и отправить пароль как есть.

use mysql_common::auth::plugins::{AuthProc, ChallengeResponsePlugin, Context, Response};
use mysql_common::constants::{CapabilityFlags, StatusFlags};
use mysql_common::io::ParseBuf;
use mysql_common::packets::{
    AuthPlugin, AuthSwitchRequest, Column, ErrPacket, HandshakePacket, HandshakeResponse,
};
use mysql_common::proto::{MyDeserialize, MySerialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Пакет длиной ровно 0xFFFFFF продолжается следующим — так протокол передаёт большие
/// тела. Число не «магическое», это предел трёхбайтовой длины в заголовке.
const MAX_PAYLOAD: usize = 0xFF_FF_FF;

/// Соединение с базой. Живёт поверх любого потока — у нас это канал SSH.
pub struct Conn<S> {
    stream: S,
    /// Номер пакета. Сервер и клиент считают их по очереди, и рассинхрон здесь —
    /// это не «неверный ответ», а разрыв соединения без объяснений.
    seq: u8,
    capabilities: CapabilityFlags,
}

/// Что мы умеем и просим у сервера.
///
/// Сжатия и TLS здесь намеренно нет: канал уже внутри SSH, второй слой ничего не
/// добавит, а разбора добавит много.
fn wanted(server: CapabilityFlags, with_db: bool) -> CapabilityFlags {
    let mut c = CapabilityFlags::CLIENT_PROTOCOL_41
        | CapabilityFlags::CLIENT_SECURE_CONNECTION
        | CapabilityFlags::CLIENT_LONG_PASSWORD
        | CapabilityFlags::CLIENT_LONG_FLAG
        | CapabilityFlags::CLIENT_TRANSACTIONS
        | CapabilityFlags::CLIENT_PLUGIN_AUTH;
    if with_db {
        c |= CapabilityFlags::CLIENT_CONNECT_WITH_DB;
    }
    if server.contains(CapabilityFlags::CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA) {
        c |= CapabilityFlags::CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA;
    }
    if server.contains(CapabilityFlags::CLIENT_DEPRECATE_EOF) {
        c |= CapabilityFlags::CLIENT_DEPRECATE_EOF;
    }
    // Без этого флага сервер отказывается выполнять `CALL`: хранимая процедура отвечает
    // несколькими результатами, и права на такой ответ надо запросить заранее. Отказ
    // приходит словами «can.t return a result set in the given context» — человеку с
    // такой подсказкой делать нечего, а процедуры на серверах живут.
    //
    // CLIENT_MULTI_STATEMENTS при этом НЕ просим: он разрешил бы слать несколько команд
    // одной строкой, а это отдельное решение с отдельными последствиями.
    c |= CapabilityFlags::CLIENT_MULTI_RESULTS;
    c & server
}

/// Данные, на которые опирается плагин аутентификации.
struct AuthCtx {
    pass: Vec<u8>,
    scramble: Vec<u8>,
    /// Открытый ключ сервера — появляется только если плагин его запросил.
    server_key: Option<Vec<u8>>,
}

impl Context for &AuthCtx {
    fn pass(&self) -> &[u8] {
        &self.pass
    }
    /// Канал SSH — не IPC и не TLS, и врать здесь нельзя: обе эти пометки заставляют
    /// `caching_sha2_password` отправить пароль открытым текстом.
    fn is_ipc_transport(&self) -> bool {
        false
    }
    fn is_tls_transport(&self) -> bool {
        false
    }
    fn scramble(&self) -> &[u8] {
        &self.scramble
    }
    fn server_key_pem(&self) -> Option<&[u8]> {
        self.server_key.as_deref()
    }
}

/// Результат запроса в том же виде, в каком его отдают остальные базы.
pub struct QueryOut {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected: u64,
}

/// Разбор пакетов, не касающийся потока: и проверить его можно на голых байтах.
impl<S> Conn<S> {
    fn err_text(&self, p: &[u8]) -> String {
        match ErrPacket::deserialize(self.capabilities, &mut ParseBuf(p)) {
            Ok(e) => e.to_string(),
            Err(_) => "Ошибка базы без описания".to_string(),
        }
    }

    /// Флаги состояния из завершающего пакета.
    ///
    /// Нужны ради одного признака — «дальше есть ещё результат». Раскладка отличается:
    /// у старого EOF сначала идут предупреждения, у нового OK — число изменённых строк
    /// и последний идентификатор. Не разобрали — считаем, что продолжения нет: лишний
    /// круг чтения повесил бы панель, а это хуже, чем показать один результат.
    fn status_of(&self, p: &[u8]) -> StatusFlags {
        let mut b = &p[1..];
        let ok_shape =
            p.first() == Some(&0x00) || self.capabilities.contains(CapabilityFlags::CLIENT_DEPRECATE_EOF);
        if ok_shape {
            if lenenc(&mut b).is_none() || lenenc(&mut b).is_none() {
                return StatusFlags::empty();
            }
        } else {
            if b.len() < 2 {
                return StatusFlags::empty();
            }
            b = &b[2..];
        }
        if b.len() < 2 {
            return StatusFlags::empty();
        }
        StatusFlags::from_bits_truncate(u16::from_le_bytes([b[0], b[1]]))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> Conn<S> {
    /// Проходит рукопожатие и вход. Возвращает готовое к запросам соединение.
    pub async fn connect(
        stream: S,
        user: &str,
        password: &str,
        database: Option<&str>,
    ) -> Result<Self, String> {
        let mut c = Conn { stream, seq: 0, capabilities: CapabilityFlags::empty() };

        let greeting = c.read_packet().await?;
        let hs = HandshakePacket::deserialize((), &mut ParseBuf(&greeting))
            .map_err(|e| format!("Сервер ответил не по протоколу MySQL: {e}"))?;

        let plugin = hs.auth_plugin().unwrap_or(AuthPlugin::MysqlNativePassword).into_owned();
        let mut ctx = AuthCtx {
            pass: password.as_bytes().to_vec(),
            scramble: hs.nonce(),
            server_key: None,
        };
        let mut proc = AuthProc::init(&plugin).map_err(|e| {
            format!("Сервер требует способ входа, который мы не поддерживаем: {e}")
        })?;

        let first = proc
            .run(&ctx, &ctx.scramble.clone())
            .map_err(|e| format!("Не удалось подготовить вход: {e}"))?;

        c.capabilities = wanted(hs.capabilities(), database.is_some_and(|d| !d.is_empty()));
        let response = HandshakeResponse::new(
            first.data().map(|d| d.to_vec()),
            hs.server_version_parsed().unwrap_or((5, 7, 0)),
            Some(user.as_bytes()),
            database.filter(|d| !d.is_empty()).map(|d| d.as_bytes()),
            Some(plugin.borrow()),
            c.capabilities,
            None,
            MAX_PAYLOAD as u32,
        );
        let mut buf = Vec::new();
        response.serialize(&mut buf);
        c.write_packet(&buf).await?;

        c.finish_auth(&mut proc, &mut ctx).await?;
        Ok(c)
    }

    /// Доигрывает вход: смена плагина, дополнительные шаги, запрос открытого ключа.
    async fn finish_auth(&mut self, proc: &mut AuthProc, ctx: &mut AuthCtx) -> Result<(), String> {
        loop {
            let p = self.read_packet().await?;
            match p.first() {
                // Вход принят.
                Some(0x00) => return Ok(()),
                Some(0xFF) => return Err(self.err_text(&p)),
                // Сервер просит другой способ входа — начинаем с ним заново.
                Some(0xFE) => {
                    let sw = AuthSwitchRequest::deserialize((), &mut ParseBuf(&p))
                        .map_err(|e| format!("Непонятная смена способа входа: {e}"))?;
                    let plugin = sw.auth_plugin().into_owned();
                    ctx.scramble = sw.plugin_data().to_vec();
                    *proc = AuthProc::init(&plugin)
                        .map_err(|e| format!("Способ входа не поддерживается: {e}"))?;
                    let challenge = ctx.scramble.clone();
                    self.step(proc, ctx, &challenge).await?;
                }
                // Продолжение разговора: у caching_sha2 здесь приезжает и признак
                // «пароль в кэше», и сам открытый ключ.
                Some(0x01) => {
                    let challenge = p[1..].to_vec();
                    // Ответ на запрос ключа — это PEM. Запоминаем: плагин возьмёт его
                    // из контекста, когда будет шифровать пароль.
                    if challenge.starts_with(b"-----BEGIN") {
                        ctx.server_key = Some(challenge.clone());
                    }
                    self.step(proc, ctx, &challenge).await?;
                }
                _ => return Err("Неожиданный ответ сервера при входе".into()),
            }
        }
    }

    /// Один шаг плагина: посчитать ответ и отправить, если он есть.
    async fn step(
        &mut self,
        proc: &mut AuthProc,
        ctx: &AuthCtx,
        challenge: &[u8],
    ) -> Result<(), String> {
        let r: Response = proc
            .run(ctx, challenge)
            .map_err(|e| format!("Вход не удался: {e}"))?;
        if let Some(data) = r.data() {
            let data = data.to_vec();
            self.write_packet(&data).await?;
        }
        Ok(())
    }

    /// Выполняет запрос текстовым протоколом.
    pub async fn query(&mut self, sql: &str) -> Result<QueryOut, String> {
        // Каждая команда начинает счёт пакетов заново.
        self.seq = 0;
        let mut cmd = Vec::with_capacity(sql.len() + 1);
        cmd.push(0x03); // COM_QUERY
        cmd.extend_from_slice(sql.as_bytes());
        self.write_packet(&cmd).await?;

        let (out, mut status) = self.read_result().await?;

        // Хранимая процедура отвечает не одним результатом, а несколькими. Показываем
        // первый, но остальные обязаны быть вычитаны из потока: иначе следующий запрос
        // прочтёт хвост предыдущего, и соединение разъедется молча — а выглядеть это
        // будет как «база вернула ерунду».
        while status.contains(StatusFlags::SERVER_MORE_RESULTS_EXISTS) {
            let (_, next) = self.read_result().await?;
            status = next;
        }
        Ok(out)
    }

    /// Читает один результат целиком и возвращает его вместе с флагами состояния.
    async fn read_result(&mut self) -> Result<(QueryOut, StatusFlags), String> {
        let head = self.read_packet().await?;
        match head.first() {
            Some(0xFF) => return Err(self.err_text(&head)),
            Some(0x00) => {
                // Запрос без выборки: в пакете лежит число изменённых строк.
                let mut buf = &head[1..];
                let affected = lenenc(&mut buf).unwrap_or(0);
                let status = self.status_of(&head);
                return Ok((QueryOut { columns: Vec::new(), rows: Vec::new(), affected }, status));
            }
            // Сервер просит прислать локальный файл. Мы этого не делаем: команда
            // читала бы файлы с машины пользователя по указанию сервера.
            Some(0xFB) => return Err("LOAD DATA LOCAL INFILE не поддерживается".into()),
            None => return Err("Пустой ответ сервера".into()),
            _ => {}
        }

        let mut buf = &head[..];
        let count = lenenc(&mut buf).ok_or("Не удалось прочитать число колонок")? as usize;
        // Заведомо невозможное число колонок — признак того, что мы читаем не то.
        if count == 0 || count > 4096 {
            return Err(format!("Странное число колонок: {count}"));
        }

        let mut columns = Vec::with_capacity(count);
        for _ in 0..count {
            let p = self.read_packet().await?;
            let col = Column::deserialize((), &mut ParseBuf(&p))
                .map_err(|e| format!("Не разобрали описание колонки: {e}"))?;
            columns.push(col.name_str().to_string());
        }
        // Старые серверы закрывают список колонок отдельным пакетом EOF, новые — нет.
        if !self.capabilities.contains(CapabilityFlags::CLIENT_DEPRECATE_EOF) {
            let _ = self.read_packet().await?;
        }

        let mut rows = Vec::new();
        let status = loop {
            let p = self.read_packet().await?;
            if is_end(&p) {
                break self.status_of(&p);
            }
            if p.first() == Some(&0xFF) {
                return Err(self.err_text(&p));
            }
            let mut b = &p[..];
            let mut row = Vec::with_capacity(count);
            for _ in 0..count {
                row.push(lenenc_str(&mut b));
            }
            rows.push(row);
        };
        Ok((QueryOut { columns, rows, affected: 0 }, status))
    }

    /// Читает один логический пакет, склеивая продолжения.
    async fn read_packet(&mut self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        loop {
            let mut head = [0u8; 4];
            self.stream
                .read_exact(&mut head)
                .await
                .map_err(|e| format!("Соединение с базой оборвалось: {e}"))?;
            let len = u32::from_le_bytes([head[0], head[1], head[2], 0]) as usize;
            self.seq = head[3].wrapping_add(1);
            let start = out.len();
            out.resize(start + len, 0);
            self.stream
                .read_exact(&mut out[start..])
                .await
                .map_err(|e| format!("Ответ базы оборвался: {e}"))?;
            if len < MAX_PAYLOAD {
                return Ok(out);
            }
        }
    }

    /// Пишет пакет, разбивая слишком большое тело на части.
    async fn write_packet(&mut self, body: &[u8]) -> Result<(), String> {
        let mut rest = body;
        loop {
            let n = rest.len().min(MAX_PAYLOAD);
            let mut head = [0u8; 4];
            head[..3].copy_from_slice(&(n as u32).to_le_bytes()[..3]);
            head[3] = self.seq;
            self.seq = self.seq.wrapping_add(1);
            self.stream
                .write_all(&head)
                .await
                .map_err(|e| format!("Не отправили запрос: {e}"))?;
            self.stream
                .write_all(&rest[..n])
                .await
                .map_err(|e| format!("Не отправили запрос: {e}"))?;
            rest = &rest[n..];
            // Тело ровно в предел длиной требует пустого пакета следом, иначе сервер
            // будет ждать продолжения, которого не будет.
            if n < MAX_PAYLOAD {
                break;
            }
        }
        self.stream
            .flush()
            .await
            .map_err(|e| format!("Не отправили запрос: {e}"))
    }
}

/// Конец выборки: EOF у старых серверов и OK у новых. Оба короткие и начинаются с 0xFE —
/// в отличие от строки данных, где 0xFE был бы началом восьмибайтовой длины, то есть
/// поля больше шестнадцати мегабайт.
///
/// Порог в девять байт держится на том, что мы **не просим** `CLIENT_SESSION_TRACK`:
/// с ним сервер дописывает в этот же пакет сведения о сессии, и он перестаёт быть
/// коротким. Если флаг когда-нибудь понадобится, порог придётся пересматривать —
/// на это есть тест среди проверок возможностей.
fn is_end(p: &[u8]) -> bool {
    p.first() == Some(&0xFE) && p.len() < 9
}

/// Целое переменной длины. Возвращает `None`, если байтов не хватило.
fn lenenc(buf: &mut &[u8]) -> Option<u64> {
    let (&first, rest) = buf.split_first()?;
    let take = |n: usize, b: &mut &[u8]| -> Option<u64> {
        if b.len() < n {
            return None;
        }
        let mut v = [0u8; 8];
        v[..n].copy_from_slice(&b[..n]);
        *b = &b[n..];
        Some(u64::from_le_bytes(v))
    };
    *buf = rest;
    match first {
        0xFC => take(2, buf),
        0xFD => take(3, buf),
        0xFE => take(8, buf),
        // 0xFB в этом месте — NULL, но как число он не встречается.
        n if n < 0xFB => Some(n as u64),
        _ => None,
    }
}

/// Строка переменной длины. `None` — это NULL, и он не то же самое, что пустая строка.
fn lenenc_str(buf: &mut &[u8]) -> Option<String> {
    if buf.first() == Some(&0xFB) {
        *buf = &buf[1..];
        return None;
    }
    let len = lenenc(buf)? as usize;
    if buf.len() < len {
        return None;
    }
    let s = String::from_utf8_lossy(&buf[..len]).to_string();
    *buf = &buf[len..];
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn целое_переменной_длины_читается_по_первому_байту() {
        assert_eq!(lenenc(&mut &[10u8][..]), Some(10));
        assert_eq!(lenenc(&mut &[0xFC, 0x01, 0x01][..]), Some(257));
        assert_eq!(lenenc(&mut &[0xFD, 0x01, 0x00, 0x01][..]), Some(65537));
    }

    #[test]
    fn обрезанное_целое_не_превращается_в_ноль() {
        // Ноль здесь означал бы «ноль строк», а на деле мы просто не дочитали пакет.
        assert_eq!(lenenc(&mut &[0xFD, 0x01][..]), None);
        assert_eq!(lenenc(&mut &[][..]), None);
    }

    #[test]
    fn null_и_пустая_строка_различаются() {
        // В таблице это разные вещи, и путать их нельзя ни на одной базе.
        assert_eq!(lenenc_str(&mut &[0xFB][..]), None);
        assert_eq!(lenenc_str(&mut &[0x00][..]), Some(String::new()));
    }

    #[test]
    fn строка_читается_целиком_и_двигает_курсор() {
        let data = [0x03, b'a', b'b', b'v', 0x01, b'x'];
        let mut b = &data[..];
        assert_eq!(lenenc_str(&mut b), Some("abv".into()));
        // Курсор обязан встать на следующее поле, иначе вся строка поедет.
        assert_eq!(lenenc_str(&mut b), Some("x".into()));
        assert!(b.is_empty());
    }

    #[test]
    fn конец_выборки_отличается_от_строки_данных() {
        // 0xFE в начале длинного пакета — это длина, а не конец: перепутать значит
        // оборвать выдачу на первой же строке с большим полем.
        assert!(is_end(&[0xFE, 0x00, 0x00, 0x02, 0x00]));
        assert!(!is_end(&[0xFE, 1, 2, 3, 4, 5, 6, 7, 8, 9]));
        assert!(!is_end(&[0x00]));
    }

    #[test]
    fn у_нас_не_просят_ни_сжатия_ни_tls() {
        // Канал уже внутри SSH. Лишний слой шифрования не добавил бы защиты, зато
        // добавил бы разбор сертификатов базы и её настройку.
        let server = CapabilityFlags::all();
        let c = wanted(server, false);
        assert!(!c.contains(CapabilityFlags::CLIENT_COMPRESS));
        assert!(!c.contains(CapabilityFlags::CLIENT_SSL));
        assert!(c.contains(CapabilityFlags::CLIENT_PROTOCOL_41));
        // Без имени базы флаг не просим — иначе сервер будет ждать его в пакете.
        assert!(!c.contains(CapabilityFlags::CLIENT_CONNECT_WITH_DB));
        assert!(wanted(server, true).contains(CapabilityFlags::CLIENT_CONNECT_WITH_DB));
        // От этого флага зависит признак конца выборки: с ним завершающий пакет
        // перестаёт быть коротким, и `is_end` начнёт принимать его за строку данных.
        assert!(
            !c.contains(CapabilityFlags::CLIENT_SESSION_TRACK),
            "появился CLIENT_SESSION_TRACK — пересмотри порог в is_end"
        );
        // Несколько результатов подряд разрешаем (иначе не работает CALL), а несколько
        // команд одной строкой — нет: это разные флаги и разные последствия.
        assert!(c.contains(CapabilityFlags::CLIENT_MULTI_RESULTS));
        assert!(!c.contains(CapabilityFlags::CLIENT_MULTI_STATEMENTS));
    }

    #[test]
    fn не_просим_того_чего_сервер_не_умеет() {
        // Старая MariaDB не знает DEPRECATE_EOF; попросить — значит не дождаться пакета,
        // который сервер по-прежнему шлёт.
        let old = CapabilityFlags::CLIENT_PROTOCOL_41 | CapabilityFlags::CLIENT_SECURE_CONNECTION;
        let c = wanted(old, false);
        assert!(!c.contains(CapabilityFlags::CLIENT_DEPRECATE_EOF));
        assert!(!c.contains(CapabilityFlags::CLIENT_PLUGIN_AUTH));
    }

    #[test]
    fn флаги_состояния_читаются_из_обоих_видов_конца() {
        // EOF старого образца: 0xFE, предупреждения, флаги.
        let old_conn = Conn { stream: (), seq: 0, capabilities: CapabilityFlags::CLIENT_PROTOCOL_41 };
        let eof = [0xFE, 0x00, 0x00, 0x08, 0x00];
        assert!(old_conn.status_of(&eof).contains(StatusFlags::SERVER_MORE_RESULTS_EXISTS));

        // OK нового образца: 0xFE, изменённые строки, идентификатор, флаги.
        let new_conn = Conn {
            stream: (),
            seq: 0,
            capabilities: CapabilityFlags::CLIENT_DEPRECATE_EOF,
        };
        let ok = [0xFE, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00];
        assert!(new_conn.status_of(&ok).contains(StatusFlags::SERVER_MORE_RESULTS_EXISTS));

        // Продолжения нет — и выдумывать его нельзя, иначе повиснем на лишнем чтении.
        let done = [0xFE, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(!new_conn.status_of(&done).contains(StatusFlags::SERVER_MORE_RESULTS_EXISTS));
    }

    #[test]
    fn обрезанный_конец_не_выдумывает_продолжение() {
        let c = Conn { stream: (), seq: 0, capabilities: CapabilityFlags::CLIENT_DEPRECATE_EOF };
        assert_eq!(c.status_of(&[0xFE]), StatusFlags::empty());
        assert_eq!(c.status_of(&[0xFE, 0x00]), StatusFlags::empty());
    }
}
