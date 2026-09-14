//! SCP поверх SSH exec (`scp -f` / `scp -t`) и ls/exec для каталогов без SFTP.

use crate::sftp::{check_remote_path, dup_key, emit_transfer, join_remote, TransferHub, CANCELLED};
use crate::ssh::{ClientHandler, SharedHandle};
use base64::{engine::general_purpose::STANDARD, Engine};
use russh::client;
use russh::{Channel, ChannelMsg};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::AppHandle;
const SCP_CHUNK: usize = 64 * 1024;
const MAX_PREVIEW: u64 = 8 * 1024 * 1024;
const MAX_EDIT_SIZE: u64 = 5 * 1024 * 1024;
/// Предел служебной строки. Заголовок файла - режим, размер и имя, а имя длиннее 255 байт
/// файловые системы не держат.
const MAX_SCP_LINE: usize = 4096;
/// Как часто сообщать о ходе передачи.
const PROGRESS_STEP: u64 = 1024 * 1024;

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

struct ScpIo {
    channel: Channel<client::Msg>,
    buf: Vec<u8>,
}

impl ScpIo {
    async fn open(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, cmd: &str) -> Result<Self, String> {
        let channel = crate::ssh::open_session_channel(handle).await?;
        channel.exec(true, cmd).await.map_err(|e| e.to_string())?;
        Ok(Self { channel, buf: Vec::new() })
    }

    /// Дочитывает в буфер следующий пакет канала.
    async fn fill(&mut self) -> Result<(), String> {
        loop {
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    self.buf.extend_from_slice(&data);
                    return Ok(());
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    return Err(format!("SCP: канал закрыт (код {exit_status})"));
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                    return Err("SCP: неожиданный конец канала".into());
                }
                _ => {}
            }
        }
    }

    async fn read_byte(&mut self) -> Result<u8, String> {
        while self.buf.is_empty() {
            self.fill().await?;
        }
        Ok(self.buf.remove(0))
    }

    /// Ответ удалённого scp: 0 - принято, 1 - предупреждение, 2 - фатальная ошибка.
    ///
    /// За кодами 1 и 2 идёт текст ошибки до перевода строки, и его обязательно нужно
    /// вычитать. Иначе поток разъезжается, и следующая проверка спотыкается о первую
    /// букву этого текста - пользователь получал «неверный ack (115)» вместо причины.
    async fn read_ack(&mut self) -> Result<(), String> {
        match self.read_byte().await? {
            0 => Ok(()),
            code @ (1 | 2) => {
                let msg = self.read_line().await.unwrap_or_default();
                let msg = msg.trim();
                Err(if msg.is_empty() {
                    format!("SCP: удалённая ошибка (код {code})")
                } else {
                    format!("SCP: {msg}")
                })
            }
            b => Err(format!("SCP: неверный ack ({b})")),
        }
    }

    async fn send_ack(&mut self) -> Result<(), String> {
        self.channel.data(&[0u8][..]).await.map_err(|e| e.to_string())
    }

    async fn read_line(&mut self) -> Result<String, String> {
        loop {
            if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
                let line = String::from_utf8_lossy(&self.buf[..i]).to_string();
                self.buf.drain(..=i);
                return Ok(line);
            }
            // Сервер, не присылающий перевода строки, иначе растил бы буфер до конца памяти.
            if self.buf.len() > MAX_SCP_LINE {
                return Err("SCP: служебная строка без конца - поток не похож на SCP".into());
            }
            self.fill().await?;
        }
    }

    async fn write_all(&mut self, data: &[u8]) -> Result<(), String> {
        for chunk in data.chunks(SCP_CHUNK) {
            self.channel.data(chunk).await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn perm_to_mode(perm: &str) -> u32 {
    if perm.len() < 10 {
        return 0o644;
    }
    fn tri(p: &str, i: usize) -> u32 {
        let mut m = 0u32;
        let bytes = p.as_bytes();
        if bytes.get(i).copied() == Some(b'r') {
            m |= 4;
        }
        if bytes.get(i + 1).copied() == Some(b'w') {
            m |= 2;
        }
        let x = bytes.get(i + 2).copied();
        if x == Some(b'x') || x == Some(b's') || x == Some(b't') {
            m |= 1;
        }
        m
    }
    tri(perm, 1) * 64 + tri(perm, 4) * 8 + tri(perm, 7)
}

/// Разбор строки `ls -lan`.
pub fn parse_ls_line(line: &str) -> Option<(char, u64, u32, u64, String, Option<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("total ") {
        return None;
    }
    let kind = line.chars().next()?;
    if kind == 't' {
        return None;
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 9 {
        return None;
    }
    let size: u64 = parts[4].parse().ok()?;
    let mode = perm_to_mode(parts[0]);
    let mtime = 0u64;
    let name_part = parts[8..].join(" ");
    if name_part == "." || name_part == ".." {
        return None;
    }
    let (name, target) = if let Some((n, t)) = name_part.split_once(" -> ") {
        (n.trim().to_string(), Some(t.trim().to_string()))
    } else {
        (name_part, None)
    };
    Some((kind, size, mode, mtime, name, target))
}

async fn canonical_dir(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str) -> Result<String, String> {
    let p = if path.is_empty() || path == "." {
        "/".to_string()
    } else {
        path.to_string()
    };
    check_remote_path(&p)?;
    if p == "/" {
        return Ok("/".into());
    }
    let cmd = format!("cd -- {} && pwd -P", shell_quote(&p));
    let (code, out, err) = crate::ssh::exec(handle, &cmd, None).await?;
    if code != 0 {
        return Err(if err.trim().is_empty() {
            format!("Каталог «{p}» недоступен (код {code})")
        } else {
            err.trim().to_string()
        });
    }
    Ok(out.trim().to_string())
}

async fn run_sh(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, script: &str) -> Result<(), String> {
    let (code, _out, err) = crate::ssh::exec(handle, script, None).await?;
    if code != 0 {
        return Err(if err.trim().is_empty() {
            format!("Команда завершилась с кодом {code}")
        } else {
            err.trim().to_string()
        });
    }
    Ok(())
}

pub async fn list(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str) -> Result<Value, String> {
    let abs = canonical_dir(handle, path).await?;
    let cmd = format!("LC_ALL=C ls -lan -- {}", shell_quote(&abs));
    let (code, out, err) = crate::ssh::exec(handle, &cmd, None).await?;
    if code != 0 {
        return Err(if err.trim().is_empty() {
            format!("ls завершился с кодом {code}")
        } else {
            err.trim().to_string()
        });
    }
    let mut entries: Vec<Value> = Vec::new();
    for line in out.lines() {
        let Some((kind, size, mode, mtime, name, target)) = parse_ls_line(line) else {
            continue;
        };
        let entry_type = match kind {
            'd' => "dir",
            'l' => "link",
            '-' => "file",
            _ => continue,
        };
        let (link_type, target_val) = if entry_type == "link" {
            (
                target.as_ref().map(|_| "file"),
                target,
            )
        } else {
            (None, None)
        };
        entries.push(json!({
            "name": name,
            "type": entry_type,
            "size": size,
            "mtime": mtime,
            "mode": mode,
            "target": target_val,
            "linkType": link_type,
        }));
    }
    entries.sort_by(|a, b| {
        let ad = a["type"] == json!("dir");
        let bd = b["type"] == json!("dir");
        bd.cmp(&ad).then_with(|| {
            a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or(""))
        })
    });
    Ok(json!({ "path": abs, "entries": entries, "backend": "scp" }))
}

/// Заголовок файла `C<режим> <размер> <имя>`: режим и размер.
///
/// Размер - недоверенное число с сервера. Память по нему не выделяется, а `limit` отсекает
/// слишком большой файл ещё до передачи данных.
fn parse_file_header(line: &str, limit: Option<u64>) -> Result<(u32, u64), String> {
    if line == "\x04" || line.is_empty() {
        return Err("SCP: файл не передан".into());
    }
    let Some(rest) = line.strip_prefix('C') else {
        return Err(format!("SCP: ожидали файл, получили «{line}»"));
    };
    let mut parts = rest.trim().splitn(3, ' ');
    let mode = parts
        .next()
        .and_then(|m| u32::from_str_radix(m, 8).ok())
        .unwrap_or(0o644)
        & 0o7777;
    let size: u64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("SCP: неверный размер в «{line}»"))?;
    if let Some(limit) = limit.filter(|l| size > *l) {
        return Err(format!("SCP: файл {size} байт больше предела {limit} байт"));
    }
    Ok((mode, size))
}

/// Принимает один файл в `sink` по мере прихода: в памяти не больше пакета канала, сколько
/// бы ни весил файл. Отмена проверяется на каждом куске.
async fn recv_file<W>(
    io: &mut ScpIo,
    sink: &mut W,
    limit: Option<u64>,
    live: &(dyn Fn() -> bool + Send + Sync),
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<u64, String>
where
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    use tokio::io::AsyncWriteExt as _;
    // В режиме `scp -f` первым говорит клиент: удалённый scp молчит, пока не получит
    // нулевой байт. Лишнее ожидание ответа здесь ставило обе стороны ждать друг друга,
    // и скачивание висело до конца сессии - молча, без ошибки.
    io.send_ack().await?;
    let line = io.read_line().await?;
    let (_mode, size) = parse_file_header(&line, limit)?;
    io.send_ack().await?;
    let mut done = 0u64;
    while done < size {
        if !live() {
            return Err(CANCELLED.into());
        }
        if io.buf.is_empty() {
            io.fill().await?;
            continue;
        }
        let take = usize::try_from(size - done).map_or(io.buf.len(), |left| left.min(io.buf.len()));
        sink.write_all(&io.buf[..take]).await.map_err(|e| e.to_string())?;
        io.buf.drain(..take);
        done += take as u64;
        progress(done, size);
    }
    sink.flush().await.map_err(|e| e.to_string())?;
    if io.read_byte().await? != 0 {
        return Err("SCP: неверный терминатор данных".into());
    }
    io.send_ack().await?;
    Ok(size)
}

/// Небольшой файл целиком в память - для просмотра и редактора, с их пределом.
async fn recv_small(io: &mut ScpIo, limit: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    recv_file(io, &mut out, Some(limit), &|| true, &mut |_: u64, _: u64| {}).await?;
    Ok(out)
}

pub async fn download_file(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &str,
    local: &str,
) -> Result<(), String> {
    download_file_ctl(handle, remote, local, &|| true, &mut |_: u64, _: u64| {})
        .await
        .map(|_| ())
}

/// Скачивание с отменой и ходом передачи: файл идёт на диск кусками, мимо памяти.
pub async fn download_file_ctl(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &str,
    local: &str,
    live: &(dyn Fn() -> bool + Send + Sync),
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<u64, String> {
    check_remote_path(remote)?;
    let cmd = format!("scp -f -- {}", shell_quote(remote));
    let mut io = ScpIo::open(handle, &cmd).await?;
    if let Some(parent) = Path::new(local).parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    // Готовое имя появляется одним переименованием: оборванная запись не должна оставить
    // обрубок под именем целого файла.
    let part = crate::sftp::part_path(local);
    let mut sink = tokio::io::BufWriter::with_capacity(1024 * 1024, crate::sftp::create_part(&part).await?);
    let got = recv_file(&mut io, &mut sink, None, live, progress).await;
    drop(sink);
    let size = match got {
        Ok(n) => n,
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
    };
    tokio::fs::rename(&part, local).await.map_err(|e| {
        let _ = std::fs::remove_file(&part);
        e.to_string()
    })?;
    Ok(size)
}

/// Отдаёт один файл ровно в `size` байт, кусками. Укоротившийся во время заливки файл -
/// ошибка: сервер ждёт объявленный размер, а дописать недостающее нечем.
async fn send_file<R>(
    io: &mut ScpIo,
    name: &str,
    src: &mut R,
    size: u64,
    mode: u32,
    live: &(dyn Fn() -> bool + Send + Sync),
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<(), String>
where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    use tokio::io::AsyncReadExt as _;
    io.read_ack().await?;
    let header = format!("C{:04o} {} {}\n", mode & 0o777, size, name);
    io.write_all(header.as_bytes()).await?;
    io.read_ack().await?;
    let mut buf = vec![0u8; SCP_CHUNK];
    let mut done = 0u64;
    while done < size {
        if !live() {
            return Err(CANCELLED.into());
        }
        let want = usize::try_from(size - done).map_or(buf.len(), |left| left.min(buf.len()));
        let n = src.read(&mut buf[..want]).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("файл укоротился во время заливки".into());
        }
        io.channel.data(&buf[..n]).await.map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done, size);
    }
    io.channel.data(&[0u8][..]).await.map_err(|e| e.to_string())?;
    io.read_ack().await?;
    Ok(())
}

fn local_mode(meta: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0o644
    }
}

/// Каталог и имя файла на сервере.
fn split_remote(remote: &str) -> (String, String) {
    let parent = Path::new(remote)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".into());
    let name = Path::new(remote)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    (parent, name)
}

/// Временное имя на сервере рядом с целевым файлом. Своё на каждую заливку; длинное имя
/// укорачивается, чтобы временное уложилось в 255 байт.
fn remote_part_name(name: &str) -> String {
    let mut end = name.len().min(200);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let tag: String = uuid::Uuid::new_v4().simple().to_string().chars().take(8).collect();
    format!(".{}.serein-{tag}.part", &name[..end])
}

/// Команда, которая ставит залитый временный файл на место целевого.
///
/// Замена - одним `mv`, поэтому оборванная раньше заливка оставляет оригинал целым. Права
/// существующего файла сохраняются, как было при записи прямо в него; время правки
/// переносится со своего файла. Ссылка разрешается до замены, иначе `mv` заменил бы саму
/// ссылку обычным файлом, а каталог с тем же именем не трогаем: `mv` молча положил бы файл
/// внутрь него.
fn finish_upload_script(tmp: &str, remote: &str, mtime_secs: Option<u64>) -> String {
    let t = shell_quote(tmp);
    let touch = mtime_secs
        .map(|s| format!("touch -m -d @{s} -- {t} 2>/dev/null; "))
        .unwrap_or_default();
    format!(
        "r={r}; \
         if [ -L \"$r\" ] && ! r=$(readlink -f -- \"$r\"); then echo 'ссылку на сервере не разрешить' >&2; exit 1; fi; \
         if [ -d \"$r\" ]; then echo 'на сервере с этим именем каталог' >&2; exit 1; fi; \
         if [ -e \"$r\" ]; then m=$(stat -c %a -- \"$r\" 2>/dev/null || stat -f %Lp \"$r\" 2>/dev/null) && chmod \"$m\" {t} 2>/dev/null; fi; \
         {touch}mv -f -- {t} \"$r\"",
        r = shell_quote(remote),
    )
}

/// Заливка во временный файл рядом с целевым и замена одним `mv`.
///
/// Раньше `scp -t` писал прямо в целевой файл: обрыв посреди передачи оставлял на сервере
/// половину нового содержимого под именем старого.
#[allow(clippy::too_many_arguments)]
async fn put_via_temp<R>(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &str,
    src: &mut R,
    size: u64,
    mode: u32,
    mtime_secs: Option<u64>,
    live: &(dyn Fn() -> bool + Send + Sync),
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<(), String>
where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    check_remote_path(remote)?;
    let (parent, name) = split_remote(remote);
    let tmp_name = remote_part_name(&name);
    let tmp = join_remote(&parent, &tmp_name);
    let sent = async {
        let mut io = ScpIo::open(handle, &format!("scp -t -- {}", shell_quote(&parent))).await?;
        send_file(&mut io, &tmp_name, src, size, mode, live, progress).await
    }
    .await;
    let placed = match sent {
        Ok(()) => run_sh(handle, &finish_upload_script(&tmp, remote, mtime_secs)).await,
        Err(e) => Err(e),
    };
    if placed.is_err() {
        // Прежний файл не тронут - убираем только свой временный.
        let _ = run_sh(handle, &format!("rm -f -- {}", shell_quote(&tmp))).await;
    }
    placed
}

pub async fn put_file(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    local: &str,
    remote: &str,
) -> Result<(), String> {
    put_file_ctl(handle, local, remote, &|| true, &mut |_: u64, _: u64| {})
        .await
        .map(|_| ())
}

/// Заливка своего файла с отменой и ходом передачи. Файл читается кусками, а не целиком.
pub async fn put_file_ctl(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    local: &str,
    remote: &str,
    live: &(dyn Fn() -> bool + Send + Sync),
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<u64, String> {
    let file = tokio::fs::File::open(local).await.map_err(|e| e.to_string())?;
    let meta = file.metadata().await.map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("«{local}» - не файл"));
    }
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let mut src = tokio::io::BufReader::with_capacity(SCP_CHUNK, file);
    put_via_temp(handle, remote, &mut src, size, local_mode(&meta), mtime, live, progress).await?;
    Ok(size)
}

pub async fn mkdir(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str) -> Result<(), String> {
    check_remote_path(path)?;
    run_sh(handle, &format!("mkdir -p -- {}", shell_quote(path))).await
}

pub async fn remove(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str, is_dir: bool) -> Result<(), String> {
    check_remote_path(path)?;
    if is_dir {
        run_sh(handle, &format!("rm -rf -- {}", shell_quote(path))).await
    } else {
        run_sh(handle, &format!("rm -f -- {}", shell_quote(path))).await
    }
}

pub async fn rename(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    from: &str,
    to: &str,
) -> Result<(), String> {
    check_remote_path(from)?;
    check_remote_path(to)?;
    run_sh(
        handle,
        &format!("mv -- {} {}", shell_quote(from), shell_quote(to)),
    )
    .await
}

pub async fn chmod(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, path: &str, mode: u32) -> Result<(), String> {
    check_remote_path(path)?;
    run_sh(
        handle,
        &format!("chmod {:o} -- {}", mode & 0o777, shell_quote(path)),
    )
    .await
}

pub async fn preview(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, remote: &str) -> Result<Value, String> {
    check_remote_path(remote)?;
    let cmd = format!("wc -c < {}", shell_quote(remote));
    let (code, out, err) = crate::ssh::exec(handle, &cmd, None).await?;
    if code != 0 {
        return Err(err.trim().to_string());
    }
    let size: u64 = out.trim().parse().unwrap_or(0);
    if size > MAX_PREVIEW {
        return Ok(json!({ "kind": "tooLarge", "size": size }));
    }
    let cmd = format!("scp -f -- {}", shell_quote(remote));
    let mut io = ScpIo::open(handle, &cmd).await?;
    let buf = recv_small(&mut io, MAX_PREVIEW).await?;
    Ok(json!({
        "kind": "bytes",
        "size": size,
        "base64": STANDARD.encode(&buf)
    }))
}

pub async fn read_file(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, remote: &str) -> Result<Value, String> {
    check_remote_path(remote)?;
    let cmd = format!("wc -c < {}", shell_quote(remote));
    let (code, out, _err) = crate::ssh::exec(handle, &cmd, None).await?;
    let size: u64 = if code == 0 { out.trim().parse().unwrap_or(0) } else { 0 };
    let mode = 0o644u32;
    let mtime = 0u64;
    if size > MAX_EDIT_SIZE {
        return Ok(json!({ "content": "", "eol": "lf", "mode": mode, "mtime": mtime, "tooLarge": true }));
    }
    let cmd = format!("scp -f -- {}", shell_quote(remote));
    let mut io = ScpIo::open(handle, &cmd).await?;
    let buf = recv_small(&mut io, MAX_EDIT_SIZE).await?;
    if buf.iter().take(8192).any(|b| *b == 0) {
        return Ok(json!({ "content": "", "eol": "lf", "mode": mode, "mtime": mtime, "binary": true }));
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let eol = if text.contains("\r\n") { "crlf" } else { "lf" };
    let content = if eol == "crlf" { text.replace("\r\n", "\n") } else { text };
    Ok(json!({ "content": content, "eol": eol, "mode": mode, "mtime": mtime }))
}

pub async fn write_file(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &str,
    content: &str,
    mode: u32,
    base_mtime: u64,
    eol: &str,
) -> Result<Value, String> {
    check_remote_path(remote)?;
    let _ = base_mtime;
    let data = if eol == "crlf" {
        content.replace('\n', "\r\n")
    } else {
        content.to_string()
    };
    // Через временный файл и одну замену: оборванная запись не оставит полфайла, а права
    // правленого файла сохраняются.
    let mut src = data.as_bytes();
    put_via_temp(handle, remote, &mut src, data.len() as u64, mode, None, &|| true, &mut |_: u64, _: u64| {}).await?;
    Ok(json!({ "ok": true, "mtime": base_mtime }))
}

pub async fn name_conflicts(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote_dir: &str,
    names: &[String],
) -> Result<Vec<String>, String> {
    let listed = list(handle, remote_dir).await?;
    let existing: std::collections::HashSet<String> = listed["entries"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|e| e["name"].as_str().map(str::to_string))
        .collect();
    Ok(names
        .iter()
        .filter_map(|raw| {
            let norm = raw.replace('\\', "/");
            let name = norm.rsplit('/').next()?.to_string();
            if name.is_empty() || name == "." || name == ".." {
                return None;
            }
            existing.contains(&name).then_some(name)
        })
        .collect())
}

pub async fn upload_path(
    app: AppHandle,
    handle: SharedHandle,
    session_id: &str,
    local: &str,
    remote_dir: &str,
    alive: Arc<AtomicBool>,
    hub: TransferHub,
) -> Result<(), String> {
    if !alive.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    check_remote_path(remote_dir)?;
    let local = local.replace('\\', "/");
    let root_name = Path::new(&local)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let mut files: Vec<(String, String, String, u64)> = Vec::new();
    crate::sftp::collect_local(&local, &join_remote(remote_dir, &root_name), &root_name, &mut files, Some(alive.as_ref()))
        .await?;
    for (lp, rp, rel, size) in files {
        if !alive.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let key = dup_key(session_id, "upload", &lp, &rp);
        let id = uuid::Uuid::new_v4().to_string();
        let Some(ctrl) = hub.start(&id, session_id, key) else {
            continue;
        };
        emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "active", None);
        let result = {
            let live = || ctrl.is_live() && alive.load(Ordering::Relaxed);
            let mut last = 0u64;
            let mut progress = |done: u64, total: u64| {
                if done - last >= PROGRESS_STEP {
                    last = done;
                    emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, total, done, "active", None);
                }
            };
            put_file_ctl(handle.as_ref(), &lp, &rp, &live, &mut progress).await
        };
        hub.finish(&id);
        if !ctrl.is_live() {
            emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "canceled", None);
            return Err(CANCELLED.into());
        }
        match result {
            Ok(_) => emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, size, "done", None),
            Err(e) if e == CANCELLED => {
                emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "canceled", None);
                return Err(e);
            }
            Err(e) => {
                emit_transfer(&app, &id, session_id, "upload", &lp, &rp, &rel, size, 0, "error", Some(&e));
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Обходит удалённое дерево через `ls`: задания на скачивание и то, что скачано не будет.
///
/// Пределы и отказы - те же, что у SFTP. Раньше здесь не было ни предела глубины и числа
/// файлов, ни отчёта о пропущенном: небезопасные имена и ссылки исчезали из скачанной папки
/// молча, и она выглядела скачанной целиком.
pub async fn walk_remote(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &str,
    local_root: &str,
    rel: &str,
    alive: Option<&AtomicBool>,
) -> Result<(Vec<(String, String, String, u64)>, Vec<(String, String)>), String> {
    let mut jobs = Vec::new();
    let mut refused = Vec::new();
    collect_remote_list(handle, remote, local_root, rel, &mut jobs, &mut refused, alive, 0).await?;
    Ok((jobs, refused))
}

#[allow(clippy::too_many_arguments)]
fn collect_remote_list<'a>(
    handle: &'a tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &'a str,
    local: &'a str,
    rel: &'a str,
    out: &'a mut Vec<(String, String, String, u64)>,
    refused: &'a mut Vec<(String, String)>,
    alive: Option<&'a AtomicBool>,
    depth: usize,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
    Box::pin(async move {
        if depth > crate::sftp::MAX_WALK_DEPTH {
            return Err(crate::sftp::walk_too_big());
        }
        let listed = list(handle, remote).await?;
        let abs = listed["path"].as_str().unwrap_or(remote).to_owned();
        let entries = listed["entries"].as_array().cloned().unwrap_or_default();
        for entry in entries {
            if alive.is_some_and(|a| !a.load(Ordering::Relaxed)) {
                return Err("Сессия закрыта".into());
            }
            if out.len() >= crate::sftp::MAX_WALK_ENTRIES {
                return Err(crate::sftp::walk_too_big());
            }
            let name = entry["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            let r = format!("{rel}/{name}");
            // Имя с сервера - не путь. Что бывает иначе, объяснено в `localname`.
            if let Err(why) = crate::localname::safe_component(&name) {
                refused.push((r, why));
                continue;
            }
            let rp = join_remote(&abs, &name);
            let lp = format!("{local}/{name}");
            match entry["type"].as_str() {
                Some("dir") => {
                    collect_remote_list(handle, &rp, &lp, &r, out, refused, alive, depth + 1).await?
                }
                Some("file") => out.push((lp, rp, r, entry["size"].as_u64().unwrap_or(0))),
                // `ls` не говорит, куда ведёт ссылка - на файл или на каталог, а заходить в
                // каталог по ссылке нельзя: так обход уходит в `/proc` и по кругу.
                Some("link") => refused.push((
                    r,
                    "это ссылка - по SCP не разобрать, файл за ней или каталог; скачайте её отдельно".to_owned(),
                )),
                _ => {}
            }
        }
        Ok(())
    })
}

/// Когда файл правили последний раз, в миллисекундах.
///
/// `None` - узнать не удалось: на сервере без `stat` (встраиваемые системы, BusyBox старых
/// сборок) такой команды может не быть вовсе. Молчаливо считать это «файл не менялся»
/// нельзя, поэтому наверх уходит именно «не знаю», а решение принимает вызывающий.
pub async fn remote_mtime(
    handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>,
    remote: &str,
) -> Result<Option<u64>, String> {
    check_remote_path(remote)?;
    let q = shell_quote(remote);
    // Сначала GNU, потом BSD: ключи у них разные, а вывод одинаковый - секунды эпохи.
    let cmd = format!("stat -c %Y -- {q} 2>/dev/null || stat -f %m {q} 2>/dev/null || true");
    let (_, out, _) = crate::ssh::exec(handle, &cmd, None).await?;
    Ok(out.trim().parse::<u64>().ok().map(|s| s * 1000))
}

async fn remote_is_dir(handle: &tokio::sync::Mutex<client::Handle<ClientHandler>>, remote: &str) -> Result<bool, String> {
    let cmd = format!(
        "if [ -d {} ]; then echo dir; elif [ -f {} ]; then echo file; else echo no; fi",
        shell_quote(remote),
        shell_quote(remote)
    );
    let (code, out, err) = crate::ssh::exec(handle, &cmd, None).await?;
    if code != 0 {
        return Err(err.trim().to_string());
    }
    match out.trim() {
        "dir" => Ok(true),
        "file" => Ok(false),
        _ => Err(format!("Путь «{remote}» не найден")),
    }
}

pub async fn download_path(
    app: AppHandle,
    handle: SharedHandle,
    session_id: &str,
    remote: &str,
    local_dir: &str,
    alive: Arc<AtomicBool>,
    hub: TransferHub,
) -> Result<(), String> {
    if !alive.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    check_remote_path(remote)?;
    let local_dir = local_dir.replace('\\', "/");
    let mut jobs: Vec<(String, String, String, u64)> = Vec::new();
    let mut refused: Vec<(String, String)> = Vec::new();
    if remote_is_dir(handle.as_ref(), remote).await? {
        let base = Path::new(remote)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "download".into());
        crate::localname::safe_component(&base).map_err(|why| format!("не могу сохранить: {why}"))?;
        let local_root = format!("{local_dir}/{base}");
        (jobs, refused) = walk_remote(handle.as_ref(), remote, &local_root, &base, Some(alive.as_ref())).await?;
    } else {
        let name = Path::new(remote)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        crate::localname::safe_component(&name).map_err(|why| format!("не могу сохранить: {why}"))?;
        jobs.push((format!("{local_dir}/{name}"), remote.to_string(), name, 0));
    }
    // Вторая сеть на случай ошибки в сборке пути: за пределы выбранной папки не пишем.
    if let Some((lp, ..)) = jobs
        .iter()
        .find(|(lp, ..)| !crate::localname::under_root(Path::new(&local_dir), Path::new(lp)))
    {
        return Err(format!(
            "путь «{lp}» выходит за пределы папки скачивания - ничего не сохранено"
        ));
    }
    // Ссылка внутри папки назначения увела бы запись наружу, хотя путь как строка внутри.
    for (lp, ..) in &jobs {
        crate::localname::no_links_below(Path::new(&local_dir), Path::new(lp))
            .map_err(|e| format!("{e} - ничего не сохранено"))?;
    }
    // Отказ виден в списке передач, как и у SFTP: молча недокачанная папка выглядит как
    // скачанная целиком.
    for (rel, why) in refused {
        let id = uuid::Uuid::new_v4().to_string();
        emit_transfer(
            &app, &id, session_id, "download", "", remote, &rel, 0, 0, "error",
            Some(&format!("не сохранено: {why}")),
        );
    }
    for (lp, rp, rel, size) in jobs {
        if !alive.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        // Перед записью - ещё раз: ссылку могли подложить уже после плана.
        if let Err(e) = crate::localname::no_links_below(Path::new(&local_dir), Path::new(&lp)) {
            let id = uuid::Uuid::new_v4().to_string();
            emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "error", Some(&e));
            return Err(e);
        }
        if let Some(parent) = Path::new(&lp).parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let key = dup_key(session_id, "download", &lp, &rp);
        let id = uuid::Uuid::new_v4().to_string();
        let Some(ctrl) = hub.start(&id, session_id, key) else {
            continue;
        };
        emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "active", None);
        let result = {
            let live = || ctrl.is_live() && alive.load(Ordering::Relaxed);
            let mut last = 0u64;
            let mut progress = |done: u64, total: u64| {
                if done - last >= PROGRESS_STEP {
                    last = done;
                    emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, total, done, "active", None);
                }
            };
            download_file_ctl(handle.as_ref(), &rp, &lp, &live, &mut progress).await
        };
        hub.finish(&id);
        if !ctrl.is_live() {
            emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "canceled", None);
            return Err(CANCELLED.into());
        }
        match result {
            Ok(n) => emit_transfer(
                &app,
                &id,
                session_id,
                "download",
                &lp,
                &rp,
                &rel,
                n,
                n.max(1),
                "done",
                None,
            ),
            Err(e) if e == CANCELLED => {
                emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "canceled", None);
                return Err(e);
            }
            Err(e) => {
                emit_transfer(&app, &id, session_id, "download", &lp, &rp, &rel, size, 0, "error", Some(&e));
                return Err(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ls_file_and_dir() {
        let f = parse_ls_line("-rw-r--r--   1 0 0     1234 Aug 31 13:00 notes.txt").unwrap();
        assert_eq!(f.0, '-');
        assert_eq!(f.1, 1234);
        assert_eq!(f.4, "notes.txt");
        let d = parse_ls_line("drwxr-xr-x   2 0 0     4096 Aug 31 13:00 srv").unwrap();
        assert_eq!(d.0, 'd');
        assert_eq!(d.4, "srv");
    }

    #[test]
    fn parse_ls_symlink() {
        let l = parse_ls_line("lrwxrwxrwx   1 0 0       11 Aug 31 13:00 link -> /etc").unwrap();
        assert_eq!(l.0, 'l');
        assert_eq!(l.4, "link");
        assert_eq!(l.5.as_deref(), Some("/etc"));
    }

    #[test]
    fn заголовок_scp_с_недопустимым_размером_отвергается_до_приёма() {
        assert_eq!(parse_file_header("C0644 12 a.txt", None).unwrap(), (0o644, 12));
        let big = parse_file_header("C0644 18446744073709551615 a", Some(MAX_PREVIEW)).unwrap_err();
        assert!(big.contains("больше предела"), "{big}");
        assert!(parse_file_header("C0644 18446744073709551616 a", None).is_err(), "за пределами u64");
        assert!(parse_file_header("C0644 -1 a", None).is_err());
        assert!(parse_file_header("D0755 0 dir", None).is_err());
        assert_eq!(parse_file_header("\x04", None).unwrap_err(), "SCP: файл не передан");
    }

    #[test]
    fn временное_имя_на_сервере_своё_и_укладывается_в_предел_имени() {
        let a = remote_part_name("отчёт.txt");
        assert_ne!(a, remote_part_name("отчёт.txt"));
        assert!(a.starts_with(".отчёт.txt.serein-") && a.ends_with(".part"), "{a}");
        assert!(remote_part_name(&"я".repeat(200)).len() <= 255);
        assert!(remote_part_name(&"ab".repeat(150)).len() <= 255);
    }

    #[test]
    fn замена_на_сервере_не_кладёт_файл_в_каталог_и_экранирует_пути() {
        let s = finish_upload_script("/srv/.a'b.serein-1.part", "/srv/a'b", Some(1_700_000_000));
        assert!(s.starts_with(&format!("r={};", shell_quote("/srv/a'b"))), "{s}");
        assert!(s.contains("if [ -d \"$r\" ]"), "{s}");
        assert!(s.contains("touch -m -d @1700000000"), "{s}");
        assert!(s.ends_with(&format!("mv -f -- {} \"$r\"", shell_quote("/srv/.a'b.serein-1.part"))), "{s}");
        assert!(!finish_upload_script("/t", "/r", None).contains("touch"));
    }
}
