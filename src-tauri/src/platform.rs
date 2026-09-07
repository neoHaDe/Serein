//! Что за система на том конце: Linux, Windows или урезанное окружение вроде BusyBox.
//!
//! Нужно потому, что дальше всё расходится: `ps` и `systemctl` на Windows не существуют, а
//! на роутере с BusyBox нет половины ключей у самих `ps` и `df`. Панели сервера обязаны
//! показывать «нет такой команды» как «здесь этого нет», а не как ошибку приложения.
//!
//! Определяем по ответу, а не по догадке из имени хоста: одна короткая команда, которая
//! выполняется на любой оболочке, и разбор её вывода - тут, отдельно, под тестами.

use serde_json::{json, Value};

/// Команда-зонд.
///
/// Расчёт на то, что она даёт осмысленный ответ в любой оболочке. В POSIX сработает
/// `uname`; в `cmd.exe` его нет, зато `ver` печатает версию Windows; в PowerShell
/// отработает `$PSVersionTable`. Лишние сообщения об ошибках уходят в никуда, поэтому
/// в выводе остаётся ровно то, что удалось.
pub const PROBE_CMD: &str = concat!(
    "uname -sr 2>/dev/null; ",
    // BusyBox выдаёт себя не именем системы, а справкой своих же утилит.
    "ls --help 2>&1 | head -n 1; ",
    "ver 2>NUL; ",
    "echo %OS%"
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Linux,
    /// Linux, но с BusyBox вместо привычных утилит: команды те же, ключей меньше.
    BusyBox,
    Windows,
    /// Ответ есть, но по нему ничего не понять. Работаем как с Linux, но осторожнее.
    Unknown,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Linux => "linux",
            Kind::BusyBox => "busybox",
            Kind::Windows => "windows",
            Kind::Unknown => "unknown",
        }
    }
}

/// Разбирает ответ зонда.
///
/// Порядок проверок не случаен: BusyBox проверяется раньше Linux, потому что `uname` там
/// тоже говорит «Linux», и по одному этому слову их не различить.
pub fn detect(stdout: &str) -> (Kind, String) {
    let text = stdout.trim();
    let lower = text.to_lowercase();

    if lower.contains("busybox") {
        return (Kind::BusyBox, first_line(text));
    }
    // `%OS%` не раскрылось - значит оболочка не cmd, и такой строке верить нельзя.
    if lower.contains("windows_nt") || lower.contains("microsoft windows") {
        return (Kind::Windows, windows_version(text));
    }
    if lower.starts_with("linux") || lower.contains("\nlinux") {
        return (Kind::Linux, first_line(text));
    }
    // Прочие юниксы ведут себя как Linux в том, что нам от них нужно.
    for unix in ["darwin", "freebsd", "openbsd", "netbsd", "sunos", "aix"] {
        if lower.starts_with(unix) {
            return (Kind::Linux, first_line(text));
        }
    }
    (Kind::Unknown, first_line(text))
}

/// Версия Windows из строки `ver`: «Microsoft Windows [Version 10.0.20348.2227]».
fn windows_version(text: &str) -> String {
    for line in text.lines() {
        let l = line.trim();
        if l.to_lowercase().contains("microsoft windows") {
            return l.trim_matches(['[', ']'].as_ref()).to_string();
        }
    }
    "Windows".into()
}

fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

pub fn to_json(kind: Kind, version: &str) -> Value {
    json!({ "kind": kind.as_str(), "version": version })
}

/// Определённая платформа сессии.
///
/// Зонд стоит одного лишнего `exec`, а спрашивать его перед каждым обновлением панели -
/// это лишний круг по сети раз в пару секунд. Поэтому ответ запоминается на сессию:
/// система под ней не меняется, пока сессия жива.
static CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, (Kind, String)>>> =
    std::sync::Mutex::new(None);

pub async fn of_session(session_id: &str, handle: &crate::ssh::SharedHandle) -> (Kind, String) {
    {
        let mut g = crate::sync::lock(&CACHE);
        if let Some(found) = g.get_or_insert_with(Default::default).get(session_id) {
            return found.clone();
        }
    }
    // Зонд не должен ронять панель: не ответил - работаем как с неизвестной системой,
    // то есть по-линуксовому, но без обещаний.
    let found = match crate::ssh::exec(handle, PROBE_CMD, None).await {
        Ok((_, out, _)) => detect(&out),
        Err(_) => (Kind::Unknown, String::new()),
    };
    crate::sync::lock(&CACHE)
        .get_or_insert_with(Default::default)
        .insert(session_id.to_string(), found.clone());
    found
}

/// Забывает платформу закрытой сессии, иначе карта растёт всю жизнь приложения.
pub fn forget(session_id: &str) {
    crate::sync::lock(&CACHE)
        .get_or_insert_with(Default::default)
        .remove(session_id);
}

/// Команды, которые различаются по платформам.
///
/// Всё, что уходит на Windows, идёт через PowerShell и склеивает поля табуляцией само:
/// разбирать `Format-Table` по ширине колонок нельзя - ширина зависит от размера окна,
/// а заголовки от языка системы.
pub mod cmd {
    /// Процессы Windows.
    ///
    /// Первой строкой - общий объём памяти: `Get-Process` даёт байты рабочего набора, а
    /// в таблице колонка называется «память, %», и без знаменателя её не посчитать.
    pub const PS_WINDOWS: &str = concat!(
        "powershell -NoProfile -NonInteractive -Command \"",
        "\\\"MT`t$((Get-CimInstance Win32_OperatingSystem).TotalVisibleMemorySize)\\\"; ",
        "Get-Process | Sort-Object -Property CPU -Descending | Select-Object -First 80 ",
        "Id, ProcessName, CPU, WorkingSet | ",
        "ForEach-Object { \\\"$($_.Id)`t$($_.ProcessName)`t$($_.CPU)`t$($_.WorkingSet)\\\" }\""
    );

    /// Службы: имя, состояние, отображаемое имя.
    pub const SERVICES_WINDOWS: &str = concat!(
        "powershell -NoProfile -NonInteractive -Command \"",
        "Get-Service | ForEach-Object { \\\"$($_.Name)`t$($_.Status)`t$($_.DisplayName)\\\" }\""
    );

    /// Журнал событий вместо journalctl.
    pub const LOGS_WINDOWS: &str = concat!(
        "powershell -NoProfile -NonInteractive -Command \"",
        "Get-WinEvent -LogName System -MaxEvents 300 -ErrorAction SilentlyContinue | ",
        "ForEach-Object { \\\"$($_.TimeCreated.ToString('yyyy-MM-ddTHH:mm:ss'))`t",
        "$($_.LevelDisplayName)`t$($_.ProviderName)`t$($_.Message -replace '`r?`n',' ')\\\" }\""
    );

    /// Снимок нагрузки: процессор, память, время работы, диски и сеть - одним вызовом.
    ///
    /// Одним, потому что каждый `exec` - это отдельный круг по сети, а панель обновляется
    /// раз в несколько секунд. Средней загрузки в Windows не существует как понятия, и
    /// поэтому её здесь нет: показать вместо неё нули значило бы соврать.
    pub const SAMPLE_WINDOWS: &str = concat!(
        "powershell -NoProfile -NonInteractive -Command \"",
        "$os = Get-CimInstance Win32_OperatingSystem; ",
        "$cs = Get-CimInstance Win32_ComputerSystem; ",
        "\\\"cores`t$($cs.NumberOfLogicalProcessors)\\\"; ",
        "\\\"cpu`t$((Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average)\\\"; ",
        "\\\"mem`t$($os.TotalVisibleMemorySize)`t$($os.FreePhysicalMemory)\\\"; ",
        "\\\"uptime`t$([int]((Get-Date) - $os.LastBootUpTime).TotalSeconds)\\\"; ",
        "\\\"os`t$($os.Caption)\\\"; ",
        "\\\"kernel`t$($os.Version)\\\"; ",
        "\\\"procs`t$((Get-Process).Count)\\\"; ",
        "\\\"sysdrive`t$($env:SystemDrive)\\\"; ",
        "Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | ",
        "ForEach-Object { \\\"disk`t$($_.DeviceID)`t$($_.Size)`t$($_.FreeSpace)\\\" }; ",
        "$a = Get-NetAdapter -Physical -ErrorAction SilentlyContinue | ",
        "Where-Object { $_.Status -eq 'Up' } | Select-Object -First 1; ",
        "if ($a) { $st = Get-NetAdapterStatistics -Name $a.Name -ErrorAction SilentlyContinue; ",
        "if ($st) { \\\"net`t$($a.Name)`t$($st.ReceivedBytes)`t$($st.SentBytes)\\\" } }\""
    );

    /// Процессы на BusyBox.
    ///
    /// Тамошний `ps` - не урезанный `procps`, а другая программа: он не знает ни `--sort`,
    /// ни колонок `pcpu`/`pmem` и на такую команду отвечает справкой в поток ошибок.
    /// До этой ветки список процессов на BusyBox выходил просто пустым - без единой
    /// ошибки, что хуже честного отказа. Общий объём памяти идёт первой строкой: RSS в
    /// килобайтах сам по себе ничего не говорит.
    pub const PS_BUSYBOX: &str = concat!(
        "awk '/^MemTotal:/{print \"MT\\t\" $2}' /proc/meminfo 2>/dev/null; ",
        "ps -o pid,user,rss,stat,args 2>/dev/null | head -n 120"
    );

    /// Службы на BusyBox: systemd там нет, зато у Alpine и у роутеров есть OpenRC.
    ///
    /// `NORC` в ответе означает, что нет и его, - панель скажет об этом словами вместо
    /// пустой таблицы.
    pub const SERVICES_BUSYBOX: &str = concat!(
        "if command -v rc-status >/dev/null 2>&1; then rc-status --all 2>/dev/null; ",
        "else echo NORC; fi"
    );
}

/// Команда управления службой под конкретную систему.
///
/// Имя и действие проверяются до подстановки в строку - там же, где и для systemd, чтобы
/// правило было одно на все системы: в имени разрешены только буквы, цифры и `-_.@:`, и
/// оно не может начинаться с дефиса. Поэтому кавычки в команде PowerShell безопасны.
pub fn service_cmd(kind: Kind, name: &str, action: &str) -> Result<String, String> {
    crate::workspace::check_service(name, action)?;
    Ok(match kind {
        Kind::Windows => {
            let verb = match action {
                "start" => "Start",
                "stop" => "Stop",
                _ => "Restart",
            };
            format!("powershell -NoProfile -NonInteractive -Command \"{verb}-Service -Name '{name}'\"")
        }
        // OpenRC: действие идёт вторым словом, а не первым, как у systemctl. Разделителя
        // `--` здесь нет намеренно: имя уже проверено и с дефиса начаться не может, а
        // поддержку `--` самим `rc-service` проверить не на чем - OpenRC в стенде нет.
        Kind::BusyBox => format!("rc-service {name} {action}"),
        _ => format!("systemctl {action} -- {name}.service"),
    })
}

/// Команда завершения процесса под конкретную систему.
pub fn kill_cmd(kind: Kind, pid: u32) -> Result<String, String> {
    if pid <= 1 {
        return Err("Нельзя слать kill pid <= 1".into());
    }
    Ok(match kind {
        Kind::Windows => {
            format!("powershell -NoProfile -NonInteractive -Command \"Stop-Process -Id {pid} -Force\"")
        }
        _ => format!("kill {pid}"),
    })
}

/// Разбор ответов Windows.
///
/// Всё, что приходит из PowerShell, склеено табуляцией - это единственный разделитель,
/// который не встречается в именах служб и процессов и не зависит от языка системы.
/// Разбор по ширине колонок ломается на русской локали, а по пробелам - на «Print Spooler».
pub mod win {
    use serde_json::{json, Value};

    /// Числа с русской локали приходят с запятой в дробной части, и `parse::<f64>`
    /// на такой строке молча даёт ноль.
    fn num(s: &str) -> f64 {
        s.trim().replace(',', ".").parse().unwrap_or(0.0)
    }

    /// Строка `MT<таб>килобайты`, затем `PID<таб>имя<таб>секунды процессора<таб>байты`.
    pub fn parse_ps(stdout: &str) -> Value {
        let mut total_kb = 0.0f64;
        let mut rows: Vec<Value> = Vec::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.trim_end().split('\t').collect();
            if parts.first().map(|s| s.trim()) == Some("MT") {
                total_kb = num(parts.get(1).copied().unwrap_or(""));
                continue;
            }
            if parts.len() < 4 {
                continue;
            }
            let Ok(pid) = parts[0].trim().parse::<u32>() else { continue };
            let cpu = num(parts[2]);
            let mem_bytes = num(parts[3]);
            // Доля памяти считается от общего объёма: Windows даёт рабочий набор в
            // байтах, а в таблице колонка называется «память, %».
            let mem = if total_kb > 0.0 {
                ((mem_bytes / 1024.0 / total_kb) * 1000.0).round() / 10.0
            } else {
                0.0
            };
            rows.push(json!({
                "pid": pid,
                // Владельца процесса `Get-Process` без прав администратора не отдаёт.
                "user": "",
                "cpu": cpu,
                "mem": mem,
                "memBytes": mem_bytes,
                "stat": "",
                "cmd": parts[1].trim(),
            }));
        }
        json!({ "ok": true, "rows": rows, "platform": "windows" })
    }

    /// Строки вида `имя<таб>состояние<таб>отображаемое имя`.
    pub fn parse_services(stdout: &str) -> Value {
        let mut rows: Vec<Value> = Vec::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.trim_end().split('\t').collect();
            if parts.len() < 2 || parts[0].trim().is_empty() {
                continue;
            }
            let state = parts[1].trim();
            rows.push(json!({
                "name": parts[0].trim(),
                // Приводим к тем же словам, что и systemd: панель у нас одна на все системы.
                "active": if state.eq_ignore_ascii_case("Running") { "active" } else { "inactive" },
                "sub": state.to_lowercase(),
                "desc": parts.get(2).map(|s| s.trim()).unwrap_or(""),
            }));
        }
        json!({ "ok": true, "rows": rows, "platform": "windows" })
    }

    /// Строки вида `время<таб>уровень<таб>источник<таб>сообщение`.
    pub fn parse_logs(stdout: &str) -> Value {
        let mut lines: Vec<String> = Vec::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.trim_end().split('\t').collect();
            if parts.len() < 4 {
                continue;
            }
            // Собираем в тот же вид, что и journalctl: панель логов уже умеет его читать.
            lines.push(format!("{} {} {}: {}", parts[0], parts[1], parts[2], parts[3]));
        }
        json!({ "ok": true, "text": lines.join("\n"), "platform": "windows" })
    }

    /// Снимок нагрузки в том же виде, что отдаёт Linux-сборщик.
    ///
    /// Вид общий намеренно: панель обзора одна на все системы и не должна знать, откуда
    /// приехали числа. Разница только в том, чего на Windows нет: средней загрузки и
    /// Docker. `load` уходит нулями, и панель на Windows его не рисует.
    pub fn parse_sample(stdout: &str) -> Value {
        let mut cores = 1u64;
        let mut cpu = 0.0f64;
        let (mut total_kb, mut free_kb) = (0.0f64, 0.0f64);
        let mut uptime = 0u64;
        let (mut os, mut kernel, mut sysdrive) = (String::new(), String::new(), String::new());
        let mut procs: Option<u64> = None;
        let mut disks: Vec<(String, f64, f64)> = Vec::new();
        let (mut iface, mut rx, mut tx) = (String::new(), None, None);

        for line in stdout.lines() {
            let p: Vec<&str> = line.trim_end().split('\t').collect();
            let at = |i: usize| p.get(i).map(|s| s.trim()).unwrap_or("");
            match p.first().map(|s| s.trim()) {
                Some("cores") => cores = (num(at(1)) as u64).max(1),
                Some("cpu") => cpu = num(at(1)),
                Some("mem") if p.len() >= 3 => {
                    total_kb = num(at(1));
                    free_kb = num(at(2));
                }
                Some("uptime") => uptime = num(at(1)).max(0.0) as u64,
                Some("os") => os = at(1).to_string(),
                Some("kernel") => kernel = at(1).to_string(),
                Some("procs") => procs = Some(num(at(1)).max(0.0) as u64),
                Some("sysdrive") => sysdrive = at(1).to_string(),
                Some("disk") if p.len() >= 4 => {
                    disks.push((at(1).to_string(), num(at(2)), num(at(3))))
                }
                Some("net") if p.len() >= 4 => {
                    iface = at(1).to_string();
                    rx = Some(num(at(2)) as u64);
                    tx = Some(num(at(3)) as u64);
                }
                _ => {}
            }
        }

        let used_kb = (total_kb - free_kb).max(0.0);
        // Системный том - аналог корня в Linux. Если его не назвали, берём первый:
        // прочерк вместо занятости диска бесполезен.
        let sys = disks
            .iter()
            .find(|d| d.0.eq_ignore_ascii_case(&sysdrive))
            .or_else(|| disks.first())
            .cloned();
        let disk_pct = sys
            .as_ref()
            .map(|(_, size, free)| {
                if *size > 0.0 {
                    ((size - free) / size * 100.0).round()
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);

        let volumes: Vec<Value> = disks
            .iter()
            .filter(|(_, size, _)| *size > 0.0)
            .map(|(mount, size, free)| {
                json!({
                    "mount": mount,
                    "sizeKb": (size / 1024.0).round() as u64,
                    "usedKb": ((size - free) / 1024.0).round().max(0.0) as u64,
                    "usePct": ((size - free) / size * 100.0).round() as u32,
                })
            })
            .collect();

        let mut out = json!({
            "ok": true,
            "platform": "windows",
            "cores": cores,
            "cpuPct": cpu.round().clamp(0.0, 100.0) as u32,
            "load": [0.0, 0.0, 0.0],
            "memTotalKb": total_kb.round() as u64,
            "memUsedKb": used_kb.round() as u64,
            "diskPct": disk_pct as u32,
            "uptimeSec": uptime,
            "dockerAvailable": false,
            "volumes": volumes,
        });
        if let Some((mount, _, _)) = sys {
            out["diskLabel"] = json!(mount);
        }
        if !os.is_empty() {
            out["os"] = json!(os);
        }
        if !kernel.is_empty() {
            out["kernel"] = json!(kernel);
        }
        if let Some(n) = procs {
            out["procCount"] = json!(n);
        }
        if !iface.is_empty() {
            out["netIface"] = json!(iface);
            if let Some(n) = rx {
                out["netRxBytes"] = json!(n);
            }
            if let Some(n) = tx {
                out["netTxBytes"] = json!(n);
            }
        }
        out
    }
}

/// Разбор ответов BusyBox.
pub mod busybox {
    use serde_json::{json, Value};

    /// Строка `MT<таб>килобайты`, затем таблица `ps -o pid,user,rss,stat,args`.
    ///
    /// Заголовок таблицы отсеивается сам: в первой колонке там слово `PID`, а не число,
    /// и строка не проходит разбор идентификатора.
    pub fn parse_ps(stdout: &str) -> Value {
        let mut total_kb = 0.0f64;
        let mut rows: Vec<Value> = Vec::new();
        for line in stdout.lines() {
            if let Some((key, value)) = line.split_once('\t') {
                if key.trim() == "MT" {
                    total_kb = value.trim().parse().unwrap_or(0.0);
                    continue;
                }
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 {
                continue;
            }
            let pid: u32 = match parts[0].parse() {
                Ok(n) if n > 0 => n,
                _ => continue,
            };
            let rss: f64 = parts[2].parse().unwrap_or(0.0);
            rows.push(json!({
                "pid": pid,
                "user": parts[1],
                // Загрузку процессора BusyBox не сообщает вовсе, и ноль здесь был бы не
                // «мало», а неправдой: панель показывает на этом месте прочерк.
                "cpu": Value::Null,
                "mem": if total_kb > 0.0 {
                    json!(((rss / total_kb) * 1000.0).round() / 10.0)
                } else {
                    Value::Null
                },
                "stat": parts[3],
                "cmd": parts[4..].join(" "),
            }));
        }
        json!({
            "ok": true,
            "rows": rows,
            "platform": "busybox",
            "note": "BusyBox не сообщает загрузку процессора по процессам",
        })
    }

    /// Ответ `rc-status --all`: строки вида `имя [ started ]`, разбитые заголовками
    /// уровней запуска. Слово `NORC` означает, что OpenRC на машине нет.
    pub fn parse_services(stdout: &str) -> Value {
        let text = stdout.trim();
        if text.is_empty() || text == "NORC" {
            return json!({
                "ok": false,
                "error": "На этой системе нет ни systemd, ни OpenRC - списка служб не существует",
            });
        }
        let mut rows: Vec<Value> = Vec::new();
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with("Runlevel:") || t.starts_with("Dynamic Runlevel:") {
                continue;
            }
            let Some((name, tail)) = t.split_once('[') else { continue };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let state = tail.trim_end_matches(']').trim().to_lowercase();
            rows.push(json!({
                "name": name,
                // Те же слова, что у systemd: панель служб одна на все системы.
                "active": if state == "started" { "active" } else { "inactive" },
                "sub": state,
                "load": "loaded",
                "desc": "",
            }));
        }
        json!({ "ok": true, "rows": rows, "platform": "busybox" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn обычный_linux_узнаётся_по_uname() {
        let (k, v) = detect("Linux 6.1.0-18-amd64\n");
        assert_eq!(k, Kind::Linux);
        assert_eq!(v, "Linux 6.1.0-18-amd64");
    }

    #[test]
    fn busybox_важнее_слова_linux() {
        // `uname` на BusyBox тоже говорит «Linux», и по одному этому слову системы
        // не различить - а разница в том, что там нет половины привычных ключей.
        let out = "Linux 5.15.0\nBusyBox v1.36.1 (2024-01-01) multi-call binary.\n";
        let (k, _) = detect(out);
        assert_eq!(k, Kind::BusyBox);
    }

    #[test]
    fn windows_узнаётся_и_по_ver_и_по_переменной() {
        let (k, v) = detect("Microsoft Windows [Version 10.0.20348.2227]\nWindows_NT\n");
        assert_eq!(k, Kind::Windows);
        assert!(v.contains("10.0.20348"), "версия потерялась: {v}");

        let (k2, _) = detect("Windows_NT\n");
        assert_eq!(k2, Kind::Windows);
    }

    #[test]
    fn нераскрытая_переменная_не_считается_виндой() {
        // В POSIX-оболочке `echo %OS%` печатает саму строку «%OS%». Принять её за Windows
        // значит отправить на Linux-сервер команды PowerShell.
        let (k, _) = detect("Linux 6.1.0\n%OS%\n");
        assert_eq!(k, Kind::Linux);
    }

    #[test]
    fn прочие_юниксы_работают_как_linux() {
        assert_eq!(detect("Darwin 23.2.0\n").0, Kind::Linux);
        assert_eq!(detect("FreeBSD 14.0-RELEASE\n").0, Kind::Linux);
    }

    #[test]
    fn пустой_или_невнятный_ответ_не_выдаётся_за_систему() {
        assert_eq!(detect("").0, Kind::Unknown);
        assert_eq!(detect("что-то непонятное\n").0, Kind::Unknown);
    }

    #[test]
    fn процессы_windows_режутся_по_табуляции() {
        // По пробелам разбирать нельзя: «Google Chrome» и «Print Spooler» развалятся,
        // а по ширине колонок - ломается на русской локали.
        let out = "1234\tGoogle Chrome\t12,5\t1048576
7\tidle\t0\t4096
";
        let v = win::parse_ps(out);
        let rows = v["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["pid"], 1234);
        assert_eq!(rows[0]["cmd"], "Google Chrome");
        // Запятая в дробях приходит с русской локали, и без замены время стало бы нулём.
        assert_eq!(rows[0]["cpu"], 12.5);
        assert_eq!(rows[0]["memBytes"], 1048576.0);
    }

    #[test]
    fn мусорные_строки_windows_пропускаются() {
        // В начале вывода PowerShell может напечатать заголовок или предупреждение.
        let out = "ЗАГОЛОВОК

42\tsvchost\t1\t2048
";
        let rows = win::parse_ps(out);
        assert_eq!(rows["rows"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn службы_windows_приводятся_к_словам_systemd() {
        // Панель служб одна на все системы, и «Running» должно читаться как «active»,
        // иначе на Windows все службы выглядят остановленными.
        let out = "Spooler\tRunning\tДиспетчер печати
WSearch\tStopped\tПоиск Windows
";
        let v = win::parse_services(out);
        let rows = v["rows"].as_array().unwrap();
        assert_eq!(rows[0]["active"], "active");
        assert_eq!(rows[0]["desc"], "Диспетчер печати");
        assert_eq!(rows[1]["active"], "inactive");
    }

    #[test]
    fn журнал_событий_складывается_в_строки_как_journalctl() {
        let out = "2026-09-05T10:00:00\tОшибка\tService Control Manager\tСлужба не запустилась
";
        let v = win::parse_logs(out);
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("2026-09-05T10:00:00"), "нет времени: {text}");
        assert!(text.contains("Служба не запустилась"), "нет сообщения: {text}");
    }

    #[test]
    fn загрузка_windows_считает_память_и_диски() {
        let out = "cores\t8
cpu\t37
mem\t8000000\t2000000
uptime\t86400
os\tMicrosoft Windows Server 2022 Standard
kernel\t10.0.20348
procs\t142
sysdrive\tC:
disk\tC:\t100000000000\t40000000000
disk\tD:\t500000000000\t50000000000
net\tEthernet\t1234567890\t987654321
";
        let v = win::parse_sample(out);
        assert_eq!(v["cores"], 8);
        assert_eq!(v["cpuPct"], 37);
        // Занято = всего минус свободно, а не «сколько-то»: путаница здесь даёт
        // правдоподобные, но неверные проценты.
        assert_eq!(v["memUsedKb"], 6000000_u64);
        assert_eq!(v["uptimeSec"], 86400_u64);
        assert_eq!(v["procCount"], 142);
        assert_eq!(v["netIface"], "Ethernet");
        assert_eq!(v["netRxBytes"], 1234567890_u64);
        // Занятость диска считается по системному тому, а не по первому попавшемуся.
        assert_eq!(v["diskLabel"], "C:");
        assert_eq!(v["diskPct"], 60);
        assert_eq!(v["volumes"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn снимок_windows_совпадает_по_виду_с_linux() {
        // Панель обзора одна на все системы: она читает те же ключи, и разъезд здесь
        // не поймает ни один тип - оба конца видят JSON.
        let v = win::parse_sample("cores\t2\ncpu\t5\nmem\t1000\t400\nsysdrive\tC:\n");
        for key in [
            "ok",
            "cores",
            "cpuPct",
            "load",
            "memTotalKb",
            "memUsedKb",
            "diskPct",
            "uptimeSec",
        ] {
            assert!(v.get(key).is_some(), "нет поля {key}");
        }
        // Средней загрузки в Windows нет, и панель обязана понять это по метке.
        assert_eq!(v["platform"], "windows");
    }

    #[test]
    fn пустой_ответ_windows_не_роняет_разбор() {
        assert_eq!(win::parse_ps("")["rows"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn доля_памяти_windows_считается_от_общего_объёма() {
        // Без строки MT колонка «память, %» показывала бы ноль у всех процессов.
        let out = "MT\t8000000\n42\tsvchost\t1,5\t819200000\n";
        let v = win::parse_ps(out);
        let row = &v["rows"][0];
        assert_eq!(row["pid"], 42);
        // Запятая - русская локаль PowerShell.
        assert_eq!(row["cpu"], 1.5);
        assert_eq!(row["mem"], 10.0);
    }

    #[test]
    fn busybox_отдаёт_прочерк_вместо_выдуманной_загрузки() {
        // У BusyBox в `ps` нет колонки процессора вовсе. Ноль на этом месте читался бы
        // как «процесс простаивает» - это неправда, поэтому там null.
        let out = "MT\t8000000\nPID   USER     RSS  STAT COMMAND\n    1 root      4664 S    sshd -D\n";
        let v = busybox::parse_ps(out);
        let rows = v["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "заголовок таблицы не должен попасть в строки");
        assert!(rows[0]["cpu"].is_null());
        assert_eq!(rows[0]["pid"], 1);
        assert_eq!(rows[0]["user"], "root");
        assert_eq!(rows[0]["cmd"], "sshd -D");
        // Долю памяти посчитать можно: RSS в килобайтах и общий объём известны.
        assert_eq!(rows[0]["mem"], 0.1);
    }

    #[test]
    fn busybox_без_общего_объёма_не_придумывает_долю_памяти() {
        let v = busybox::parse_ps("  17 root      1024 S    sh\n");
        assert!(v["rows"][0]["mem"].is_null());
    }

    #[test]
    fn openrc_читается_как_список_служб() {
        let out = "Runlevel: default\n sshd     [  started  ]\n crond    [  stopped  ]\n";
        let v = busybox::parse_services(out);
        let rows = v["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "заголовок уровня запуска - не служба");
        assert_eq!(rows[0]["name"], "sshd");
        // Слова приводятся к systemd: панель служб одна на все системы.
        assert_eq!(rows[0]["active"], "active");
        assert_eq!(rows[1]["active"], "inactive");
    }

    #[test]
    fn отсутствие_openrc_объясняется_словами() {
        // Пустая таблица выглядела бы как «служб нет», а их тут просто нечем считать.
        let v = busybox::parse_services("NORC\n");
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("OpenRC"));
    }

    #[test]
    fn команда_службы_зависит_от_системы() {
        assert_eq!(
            service_cmd(Kind::Linux, "nginx", "restart").unwrap(),
            "systemctl restart -- nginx.service"
        );
        assert_eq!(
            service_cmd(Kind::BusyBox, "sshd", "start").unwrap(),
            "rc-service sshd start"
        );
        assert!(service_cmd(Kind::Windows, "Spooler", "stop")
            .unwrap()
            .contains("Stop-Service -Name 'Spooler'"));
    }

    #[test]
    fn имя_службы_с_дефиса_не_пройдёт() {
        // Иначе оно прочтётся как ключ той программы, которой его передали.
        assert!(service_cmd(Kind::BusyBox, "-f", "start").is_err());
        assert!(service_cmd(Kind::Linux, "nginx; rm -rf /", "start").is_err());
        assert!(service_cmd(Kind::Linux, "nginx", "enable").is_err());
    }

    #[test]
    fn завершение_процесса_зависит_от_системы() {
        assert_eq!(kill_cmd(Kind::Linux, 42).unwrap(), "kill 42");
        assert!(kill_cmd(Kind::Windows, 42).unwrap().contains("Stop-Process -Id 42"));
        // Первый процесс - это init: снимать его нельзя ни на одной системе.
        assert!(kill_cmd(Kind::Windows, 1).is_err());
    }
}
