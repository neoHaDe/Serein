//! `ProxyCommand` — подключение через внешнюю программу-посредника.
//!
//! Так работает OpenSSH: вместо TCP-соединения запускается процесс, и его stdin/stdout
//! становятся каналом до сервера. Через это ходят в закрытые сети — `cloudflared access`,
//! `corkscrew` сквозь HTTP-прокси, `socat`, `ssh -W` до бастиона.
//!
//! Нам это ничего не стоит: russh умеет работать поверх любого потока
//! (`client::connect_stream`), поэтому достаточно склеить пару труб процесса в один объект.

use std::pin::Pin;
use std::process::Stdio;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// Труба до процесса-посредника: читаем его stdout, пишем в его stdin.
pub struct ProxyStream {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl AsyncRead for ProxyStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdout).poll_read(cx, buf)
    }
}

impl AsyncWrite for ProxyStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stdin).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdin).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdin).poll_shutdown(cx)
    }
}

impl Drop for ProxyStream {
    fn drop(&mut self) {
        // Иначе посредник переживёт сессию и останется висеть в процессах.
        let _ = self.child.start_kill();
    }
}

/// Подставляет токены OpenSSH: `%h` — хост, `%p` — порт, `%r` — пользователь, `%%` — сам процент.
/// Разбор идёт одним проходом, чтобы подставленное значение (например, хост с `%p` в имени)
/// не подставлялось повторно.
pub fn expand_tokens(template: &str, host: &str, port: u16, user: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('h') => out.push_str(host),
            Some('p') => out.push_str(&port.to_string()),
            Some('r') => out.push_str(user),
            Some('%') => out.push('%'),
            // Неизвестный токен оставляем как есть — пусть команда сама решает, что с ним делать.
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// Найти в команде путь, записанный для другой системы.
///
/// Ищем только заведомо чужую форму: на Unix — `C:\…`, на Windows — абсолютный юниксовый
/// путь к программе. Не пытаемся быть умнее: команда — это строка для оболочки, и любая
/// догадка сверх этого будет чаще мешать, чем помогать.
fn foreign_program_path(line: &str) -> Option<String> {
    for tok in line.split_whitespace() {
        let t = tok.trim_matches(['"', '\'']);
        if t.len() > 2 {
            let b = t.as_bytes();
            let windows_style =
                b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
            if cfg!(windows) {
                // На Windows чужим считаем только явный путь к программе, а не любой
                // аргумент вида `/home/...`: их полно в нормальных командах.
                if t.starts_with('/') && (t.ends_with("ssh") || t.contains("/bin/")) {
                    return Some(t.to_string());
                }
            } else if windows_style {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// Запускает посредника и отдаёт поток для russh.
pub fn spawn(command: &str, host: &str, port: u16, user: &str) -> Result<ProxyStream, String> {
    let line = expand_tokens(command, host, port, user);
    if line.trim().is_empty() {
        return Err("Пустая команда прокси".into());
    }
    // Профиль мог приехать с другой системы вместе с бэкапом. `C:\…\plink.exe` на Linux
    // не запустится, и оболочка ответит «команда не найдена» — по такой ошибке причину
    // не угадать. Соответствия для внешней программы мы подобрать не можем, поэтому
    // говорим прямо, что менять.
    if let Some(bad) = foreign_program_path(&line) {
        return Err(format!(
            "Команда-посредник указывает на путь другой системы: {bad}. \
             Профиль перенесён с другой ОС — поправьте ProxyCommand в настройках сервера"
        ));
    }

    // Строку отдаём оболочке целиком: в ProxyCommand принято писать конвейеры и кавычки,
    // а разбирать их самим — значит расходиться с тем, как это работает в OpenSSH.
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        // У cmd.exe свой разбор командной строки, несовместимый со стандартным
        // экранированием Rust: обычный `.arg(line)` съедает внутренние кавычки, и команда
        // с путём в кавычках или вложенным `-Command "..."` приезжает искажённой
        // (проверено: до правки посредник получал обрезанную строку).
        // `/S` + кавычки вокруг всей строки заставляют cmd взять её как есть.
        let mut std_cmd = std::process::Command::new("cmd.exe");
        std_cmd.raw_arg(format!("/S /C \"{line}\""));
        std_cmd.creation_flags(CREATE_NO_WINDOW); // без этого мигает чёрное окно консоли
        Command::from(std_cmd)
    };

    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&line);
        c
    };

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // stderr посреднику оставляем: его сообщения уходят в лог приложения, а не в SSH-поток,
        // иначе диагностика («cloudflared: not found») просто пропадёт.
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Не удалось запустить прокси-команду: {e}"))?;

    let stdin = child.stdin.take().ok_or("Прокси-команда не дала stdin")?;
    let stdout = child.stdout.take().ok_or("Прокси-команда не дала stdout")?;
    Ok(ProxyStream { child, stdin, stdout })
}

#[cfg(test)]
mod tests {
    use super::{expand_tokens, foreign_program_path};

    #[test]
    fn substitutes_host_port_user() {
        assert_eq!(
            expand_tokens("ssh -W %h:%p bastion", "srv.local", 2222, "hade"),
            "ssh -W srv.local:2222 bastion"
        );
        assert_eq!(
            expand_tokens("connect %r@%h", "example.com", 22, "root"),
            "connect root@example.com"
        );
    }

    #[test]
    fn double_percent_is_literal() {
        assert_eq!(expand_tokens("echo 100%%", "h", 22, "u"), "echo 100%");
    }

    #[test]
    fn unknown_token_is_left_alone() {
        // `%d` в OpenSSH — домашний каталог; мы его не поддерживаем, но и не съедаем.
        assert_eq!(expand_tokens("x %d y", "h", 22, "u"), "x %d y");
        assert_eq!(expand_tokens("trailing %", "h", 22, "u"), "trailing %");
    }

    /// Команда с кавычками должна дойти до посредника целиком. На Windows это ловушка:
    /// обычное экранирование Rust несовместимо с разбором cmd.exe, и такая строка
    /// приезжала обрезанной — проверено живым запуском до правки на `/S /C "…"`.
    #[test]
    fn foreign_program_path_is_reported() {
        if cfg!(windows) {
            assert!(foreign_program_path("/usr/bin/ssh -W %h:%p jump").is_some());
            assert!(foreign_program_path(r"C:\Program Files\PuTTY\plink.exe -nc %h:%p").is_none());
        } else {
            let got = foreign_program_path(r"C:\Program Files\PuTTY\plink.exe -nc %h:%p");
            assert!(got.is_some(), "путь Windows должен опознаваться");
            assert!(foreign_program_path("ssh -W %h:%p jump").is_none());
            // Кавычки вокруг пути не должны мешать опознать его.
            assert!(foreign_program_path("\"C:/tools/nc.exe\" %h %p").is_some());
        }
    }

    #[test]
    fn command_with_quotes_survives_the_shell() {
        use super::spawn;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let cmd = if cfg!(windows) {
            "powershell -NoProfile -Command \"$l=[Console]::In.ReadLine(); [Console]::Out.WriteLine('echo:'+$l)\""
        } else {
            "sh -c 'read l; echo \"echo:$l\"'"
        };

        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let got = rt.block_on(async {
            let mut s = spawn(cmd, "srv.example", 2222, "hade").expect("посредник запустился");
            s.write_all(b"ping\r\n").await.expect("запись");
            s.flush().await.ok();
            let mut buf = vec![0u8; 128];
            let n = tokio::time::timeout(std::time::Duration::from_secs(20), s.read(&mut buf))
                .await
                .expect("посредник ответил вовремя")
                .expect("чтение");
            String::from_utf8_lossy(&buf[..n]).trim().to_string()
        });
        assert_eq!(got, "echo:ping", "команда должна дойти без потери кавычек");
    }

    #[test]
    fn substituted_value_is_not_expanded_again() {
        // Если имя хоста само содержит «%p», второй проход испортил бы команду.
        assert_eq!(
            expand_tokens("go %h", "weird%phost", 22, "u"),
            "go weird%phost"
        );
    }
}
