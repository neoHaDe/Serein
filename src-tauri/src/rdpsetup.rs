//! Что нужно, чтобы на сервере появился рабочий стол по RDP: проверить, поставить, запустить.
//!
//! Сосед `vncsetup` делает то же для VNC, и разделены они не из любви к симметрии.
//! Различий по существу три. Программа одна и та же на всех системах (`xrdp`), но
//! Debian и Fedora к ней докладывают ещё и `xorgxrdp` - без него сеанс поднимется
//! пустым. Порт один и тот же всегда (3389), а не по одному на экран. И пароля своего
//! у RDP нет вовсе: вход идёт по системной учётной записи, поэтому задавать здесь
//! нечего - и это, пожалуй, главное его удобство.
//!
//! Всё, что меняет систему, требует пароля `sudo`. Пароль **не подставляется в команду**:
//! строка команды целиком видна в списке процессов сервера любому, кто там есть. Он
//! уходит на стандартный ввод `sudo -S` - см. `ssh::exec_with_input`. Нигде не хранится.

use serde_json::{json, Value};

/// Что уже есть на сервере.
///
/// Смотрим то же, что и для VNC, и по той же причине: «RDP не работает» бывает по трём
/// разным поводам - программы нет, программа есть но не запущена, запущена но слушает
/// не там. Ответ человеку в каждом случае свой.
pub const DETECT_CMD: &str = concat!(
    "for b in xrdp xrdp-sesman; do ",
    "  p=$(command -v $b 2>/dev/null) && echo \"BIN:$b|$p\"; ",
    "done; ",
    // Кто слушает порт RDP. Он всегда один, в отличие от VNC с его экранами.
    "if command -v ss >/dev/null 2>&1; then ",
    "  ss -ltn 2>/dev/null | awk '$4 ~ /:3389$/ {print \"PORT:\" $4}'; ",
    "elif command -v netstat >/dev/null 2>&1; then ",
    "  netstat -ltn 2>/dev/null | awk '$4 ~ /:3389$/ {print \"PORT:\" $4}'; ",
    "elif [ -r /proc/net/tcp ]; then ",
    // Ни `ss`, ни `netstat` на урезанных системах может не быть. Тогда читаем ядро:
    // 0D3D - это 3389 шестнадцатеричным, а 0A в четвёртом столбце значит «слушает».
    "  awk '$4 == \"0A\" && toupper($2) ~ /:0D3D$/ {print \"PORTHEX:\" $2}' /proc/net/tcp 2>/dev/null; ",
    "fi; ",
    // Запущена ли служба. Порт мог быть занят и кем-то другим, а служба - лежать.
    "if command -v systemctl >/dev/null 2>&1; then ",
    "  echo \"SVC:$(systemctl is-active xrdp 2>/dev/null || echo неизвестно)\"; ",
    "fi; ",
    // Есть ли графическая среда: без неё показывать будет нечего.
    "if [ -d /usr/share/xsessions ] && [ -n \"$(ls -A /usr/share/xsessions 2>/dev/null)\" ]; then ",
    "  echo 'DESK:есть'; else echo 'DESK:нет'; fi; ",
    "for m in apt-get dnf yum apk zypper pacman; do ",
    "  command -v $m >/dev/null 2>&1 && { echo \"PM:$m\"; break; }; ",
    "done; ",
    "if sudo -n true 2>/dev/null; then echo 'SUDO:без пароля'; ",
    "elif command -v sudo >/dev/null 2>&1; then echo 'SUDO:по паролю'; ",
    "else echo 'SUDO:нет'; fi"
);

/// Разбирает ответ разведки.
pub fn parse_detect(stdout: &str) -> Value {
    let mut bins: Vec<Value> = Vec::new();
    let mut ports: Vec<String> = Vec::new();
    let mut service = String::new();
    let mut desktop = String::new();
    let mut pm = String::new();
    let mut sudo = String::new();

    for line in stdout.lines() {
        let Some((tag, val)) = line.trim().split_once(':') else {
            continue;
        };
        let val = val.trim();
        match tag {
            "BIN" => {
                let (name, path) = val.split_once('|').unwrap_or((val, ""));
                bins.push(json!({ "name": name, "path": path }));
            }
            "PORT" => {
                if !ports.contains(&val.to_string()) {
                    ports.push(val.to_string());
                }
            }
            "PORTHEX" => {
                if let Some(addr) = crate::vncsetup::decode_proc_addr(val) {
                    if !ports.contains(&addr) {
                        ports.push(addr);
                    }
                }
            }
            "SVC" => service = val.to_string(),
            "DESK" => desktop = val.to_string(),
            "PM" => pm = val.to_string(),
            "SUDO" => sudo = val.to_string(),
            _ => {}
        }
    }

    // Итог одной фразой: панели нужно не перечисление фактов, а ответ, что делать дальше.
    let вывод = if !ports.is_empty() {
        "RDP уже слушает - можно подключаться"
    } else if !bins.is_empty() {
        "xrdp установлен, но не запущен"
    } else if pm.is_empty() {
        "xrdp не найден, и чем ставить пакеты - тоже неясно"
    } else {
        "xrdp на сервере нет"
    };

    json!({
        "installed": bins,
        "listening": ports,
        "service": service,
        "desktop": desktop,
        "packageManager": pm,
        "sudo": sudo,
        "summary": вывод,
        "canInstall": package_for(&pm).is_some() && sudo != "нет",
        // Запускать имеет смысл только уже установленное - и только если оно молчит.
        "canStart": !bins.is_empty() && ports.is_empty() && sudo != "нет",
    })
}

/// Какие пакеты ставить этим менеджером.
///
/// Одного `xrdp` мало на Debian и Fedora: рабочий стол там показывает `xorgxrdp`, и без
/// него вход заканчивается пустым серым экраном - неполадка, которую потом ищут часами.
///
/// В Arch пакета в основных хранилищах нет вовсе, он живёт в AUR. Ставить оттуда чужими
/// руками мы не будем: это сборка из исходников с чужими правилами, и делать это молча
/// за человека нельзя.
pub fn package_for(pm: &str) -> Option<&'static str> {
    match pm {
        "apt-get" => Some("xrdp xorgxrdp"),
        "dnf" | "yum" => Some("xrdp xorgxrdp"),
        "zypper" => Some("xrdp"),
        "apk" => Some("xrdp"),
        _ => None,
    }
}

/// Команда установки под конкретный менеджер пакетов.
///
/// Всё неинтерактивно: спрашивать что-либо у команды, которая идёт по каналу без
/// терминала, некому - она просто повиснет.
pub fn install_cmd(pm: &str) -> Option<String> {
    let pkg = package_for(pm)?;
    let cmd = match pm {
        "apt-get" => format!("DEBIAN_FRONTEND=noninteractive apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq {pkg}"),
        "dnf" | "yum" => format!("{pm} install -y {pkg}"),
        "apk" => format!("apk add --no-cache {pkg}"),
        "zypper" => format!("zypper --non-interactive install {pkg}"),
        _ => return None,
    };
    Some(format!("sudo -S -p '' sh -c '{cmd}' 2>&1"))
}

/// Первичная настройка: включить службу, запустить и сказать, что вышло.
///
/// `enable --now` вместо `start`: иначе после перезагрузки сервера рабочий стол молча не
/// поднимется, и это выяснится в самый неподходящий момент.
///
/// Проверка в конце не украшение. `systemctl` возвращает ноль, успев только отправить
/// запрос, а служба может упасть секундой позже - и тогда «готово» было бы неправдой.
pub const ENABLE_CMD: &str = concat!(
    "sudo -S -p '' sh -c '",
    "systemctl enable --now xrdp 2>&1; ",
    "sleep 1; ",
    "systemctl is-active xrdp",
    "' 2>&1"
);

/// Что с встроенным рабочим столом Windows.
///
/// Сценарий PowerShell; кодировать и оборачивать его будет `platform::ps`. Спрашиваем
/// четыре вещи, потому что «не подключается» бывает по четырём разным причинам: запрещено
/// в настройках, не запущена служба, закрыт межсетевой экран, никто не слушает порт.
/// Пятая строка - о правах: без администратора включить ничего нельзя, и обещать не надо.
pub const DETECT_WINDOWS: &str = concat!(
    "$k = 'HKLM:\\SYSTEM\\CurrentControlSet\\Control\\Terminal Server'; ",
    "\"deny`t$((Get-ItemProperty $k -Name fDenyTSConnections -ErrorAction SilentlyContinue).fDenyTSConnections)\"; ",
    "\"svc`t$((Get-Service TermService -ErrorAction SilentlyContinue).Status)\"; ",
    "Get-NetTCPConnection -State Listen -LocalPort 3389 -ErrorAction SilentlyContinue | ",
    "ForEach-Object { \"port`t$($_.LocalAddress)\" }; ",
    "$fw = Get-NetFirewallRule -DisplayGroup 'Remote Desktop' -ErrorAction SilentlyContinue | ",
    "Where-Object { $_.Enabled -eq 'True' } | Select-Object -First 1; ",
    "\"fw`t$(if ($fw) { 'on' } else { 'off' })\"; ",
    "$p = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent()); ",
    "\"admin`t$(if ($p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { 'yes' } else { 'no' })\""
);

/// Включает встроенный рабочий стол и проверяет, что вышло.
///
/// Три действия сразу, потому что поодиночке они бесполезны: снять запрет в настройках,
/// открыть межсетевой экран и поднять службу. В конце спрашиваем состояние - `Set-Service`
/// возвращает успех, успев только отправить запрос.
pub const ENABLE_WINDOWS: &str = concat!(
    "$k = 'HKLM:\\SYSTEM\\CurrentControlSet\\Control\\Terminal Server'; ",
    "Set-ItemProperty -Path $k -Name fDenyTSConnections -Value 0 -ErrorAction SilentlyContinue; ",
    "Enable-NetFirewallRule -DisplayGroup 'Remote Desktop' -ErrorAction SilentlyContinue; ",
    "Set-Service -Name TermService -StartupType Automatic -ErrorAction SilentlyContinue; ",
    "Start-Service -Name TermService -ErrorAction SilentlyContinue; ",
    "Start-Sleep -Seconds 1; ",
    "\"deny`t$((Get-ItemProperty $k -Name fDenyTSConnections -ErrorAction SilentlyContinue).fDenyTSConnections)\"; ",
    "\"svc`t$((Get-Service TermService -ErrorAction SilentlyContinue).Status)\""
);

/// Разбирает ответ разведки Windows.
///
/// Возвращает тот же вид, что и разбор для юниксов: панель одна, и различать системы она
/// не обязана. Ставить нечего - `canInstall` всегда ложь; включить можно, если ещё не
/// включено и есть права администратора.
pub fn parse_detect_windows(stdout: &str) -> Value {
    let (mut deny, mut svc, mut fw, mut admin) =
        (String::new(), String::new(), String::new(), String::new());
    let mut ports: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let Some((tag, val)) = line.trim().split_once('\t') else {
            continue;
        };
        let val = val.trim();
        match tag.trim() {
            "deny" => deny = val.to_owned(),
            "svc" => svc = val.to_owned(),
            "fw" => fw = val.to_owned(),
            "admin" => admin = val.to_owned(),
            "port" => {
                // Адрес `::` - это «слушает везде» в записи IPv6; для человека понятнее
                // назвать порт, а не пересказывать форму записи.
                let addr = if val == "::" {
                    "[::]".to_owned()
                } else {
                    val.to_owned()
                };
                let full = format!("{addr}:3389");
                if !ports.contains(&full) {
                    ports.push(full);
                }
            }
            _ => {}
        }
    }

    let запрещён = deny == "1";
    let служба_идёт = svc.eq_ignore_ascii_case("Running");
    let есть_права = admin == "yes";

    let вывод = if !ports.is_empty() {
        "Удалённый рабочий стол включён - можно подключаться"
    } else if запрещён {
        "Удалённый рабочий стол выключен в настройках системы"
    } else if !служба_идёт {
        "Служба удалённых рабочих столов не запущена"
    } else {
        "Порт 3389 никто не слушает"
    };

    json!({
        "installed": [{ "name": "Remote Desktop", "path": "встроен в Windows" }],
        "listening": ports,
        "service": svc,
        "firewall": fw,
        "admin": есть_права,
        "packageManager": "",
        "sudo": if есть_права { "администратор" } else { "обычный пользователь" },
        "summary": вывод,
        // Ставить на Windows нечего: рабочий стол там часть системы.
        "canInstall": false,
        "canStart": ports.is_empty() && есть_права,
        // Признак для панели: на Windows нет ни пакетов, ни sudo - вместо них права
        // администратора самой сессии, и спрашивать пароль там не у кого.
        "windows": true,
    })
}

/// Разбирает ответ включения: смотрим не на код возврата, а на итоговое состояние.
pub fn parse_enable_windows(stdout: &str) -> Value {
    let v = parse_detect_windows(stdout);
    let выключено = v["service"]
        .as_str()
        .unwrap_or("")
        .eq_ignore_ascii_case("Running");
    let разрешено = !stdout.lines().any(|l| l.trim() == "deny\t1");
    if выключено && разрешено {
        json!({ "ok": true })
    } else {
        json!({
            "ok": false,
            "error": "Не удалось включить: нужны права администратора на сервере",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn состояние_windows_читается_по_четырём_приметам() {
        // Ровно тот вывод, что даёт настоящая машина: проверено вживую.
        let v = parse_detect_windows(
            "deny\t0\nsvc\tRunning\nport\t::\nport\t0.0.0.0\nfw\ton\nadmin\tyes\n",
        );
        assert_eq!(
            v["summary"],
            "Удалённый рабочий стол включён - можно подключаться"
        );
        assert_eq!(v["listening"][0], "[::]:3389");
        assert_eq!(v["canInstall"], false, "на Windows ставить нечего");
        assert_eq!(v["canStart"], false, "уже включён");

        let выкл = parse_detect_windows("deny\t1\nsvc\tStopped\nfw\toff\nadmin\tyes\n");
        assert_eq!(
            выкл["summary"],
            "Удалённый рабочий стол выключен в настройках системы"
        );
        assert_eq!(выкл["canStart"], true);
    }

    #[test]
    fn без_прав_администратора_кнопку_не_показываем() {
        // Предлагать действие, которое заведомо не выполнится, - обман.
        let v = parse_detect_windows("deny\t1\nsvc\tStopped\nfw\toff\nadmin\tno\n");
        assert_eq!(v["canStart"], false);
        assert_eq!(v["sudo"], "обычный пользователь");
    }

    #[test]
    fn включение_проверяется_по_итогу_а_не_по_коду() {
        assert_eq!(parse_enable_windows("deny\t0\nsvc\tRunning\n")["ok"], true);
        let плохо = parse_enable_windows("deny\t1\nsvc\tStopped\n");
        assert_eq!(плохо["ok"], false);
        assert!(плохо["error"].as_str().unwrap().contains("администратора"));
    }

    #[test]
    fn три_разные_причины_различаются() {
        // «Не работает» бывает по-разному, и совет в каждом случае свой.
        let нет = parse_detect("PM:apt-get\nSUDO:по паролю\nDESK:есть\n");
        assert_eq!(нет["summary"], "xrdp на сервере нет");
        assert_eq!(нет["canInstall"], true);
        assert_eq!(нет["canStart"], false, "запускать нечего");

        let есть = parse_detect("BIN:xrdp|/usr/sbin/xrdp\nPM:apt-get\nSUDO:по паролю\n");
        assert_eq!(есть["summary"], "xrdp установлен, но не запущен");
        assert_eq!(есть["canStart"], true);

        let слушает = parse_detect("BIN:xrdp|/usr/sbin/xrdp\nPORT:0.0.0.0:3389\n");
        assert_eq!(слушает["summary"], "RDP уже слушает - можно подключаться");
        assert_eq!(слушает["canStart"], false, "уже запущено");
    }

    #[test]
    fn порт_виден_и_без_ss() {
        // На урезанных системах ни `ss`, ни `netstat` может не быть - остаётся ядро.
        // 0D3D это 3389; без этой ветки мы сказали бы «не запущен» про работающий сервер.
        let v = parse_detect("BIN:xrdp|/usr/sbin/xrdp\nPORTHEX:0100007F:0D3D\n");
        assert_eq!(v["listening"][0], "127.0.0.1:3389");
        assert_eq!(v["summary"], "RDP уже слушает - можно подключаться");
    }

    #[test]
    fn на_arch_не_обещаем_того_чего_не_сделаем() {
        // Пакета нет в основных хранилищах Arch, он в AUR - это сборка из исходников
        // чужими правилами, и делать её молча за человека нельзя.
        assert_eq!(package_for("pacman"), None);
        assert!(install_cmd("pacman").is_none());
        let v = parse_detect("PM:pacman\nSUDO:без пароля\n");
        assert_eq!(
            v["canInstall"], false,
            "предлагать то, чего не сделаем, - обман"
        );
    }

    #[test]
    fn графическая_часть_ставится_вместе_с_сервером() {
        // Один xrdp без xorgxrdp даёт пустой серый экран вместо рабочего стола -
        // неполадка, которую потом ищут часами.
        assert!(package_for("apt-get").unwrap().contains("xorgxrdp"));
        assert!(package_for("dnf").unwrap().contains("xorgxrdp"));
    }

    #[test]
    fn установка_идёт_неинтерактивно_и_с_паролем_на_вход() {
        let c = install_cmd("apt-get").unwrap();
        // Пароль в строке команды не появляется - он уйдёт на стандартный ввод.
        assert!(c.contains("sudo -S"), "{c}");
        assert!(!c.contains("password"), "{c}");
        // Спрашивать что-то у команды без терминала некому.
        assert!(c.contains("noninteractive"), "{c}");
        assert!(install_cmd("свой менеджер").is_none());
    }

    #[test]
    fn запуск_включает_службу_и_проверяет_итог() {
        // Просто `start` пережил бы только до перезагрузки, а `systemctl` возвращает
        // ноль, успев лишь отправить запрос: без проверки «готово» было бы неправдой.
        assert!(ENABLE_CMD.contains("enable --now"));
        assert!(ENABLE_CMD.contains("is-active"));
        assert!(ENABLE_CMD.contains("sudo -S"));
        assert!(
            !ENABLE_CMD.contains('\n'),
            "перевод строки сломал бы разбор команды"
        );
    }

    #[test]
    fn без_прав_ставить_нечего() {
        // Предлагать установку там, где её заведомо не выполнить, - обман.
        assert_eq!(parse_detect("PM:apt-get\nSUDO:нет\n")["canInstall"], false);
        assert_eq!(parse_detect("SUDO:по паролю\n")["canInstall"], false);
    }
}
