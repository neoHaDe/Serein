//! Что нужно, чтобы на сервере появился рабочий стол: проверить, поставить, настроить.
//!
//! Родилось из простого наблюдения: панель Desktop бесполезна, если на сервере нет VNC, а
//! узнать это можно было только по невнятной ошибке подключения. Здесь она сначала
//! смотрит, что есть, и говорит словами, чего не хватает.
//!
//! Всё, что меняет систему, требует пароля `sudo`. Пароль **не подставляется в команду**:
//! строка команды целиком видна в списке процессов сервера любому, кто там есть. Он
//! уходит на стандартный ввод `sudo -S` через тот же канал - см. `ssh::exec_with_input`.
//! Нигде не сохраняется: пришёл из формы, ушёл в канал, забыт.

use serde_json::{json, Value};

/// Что уже есть на сервере.
///
/// Смотрим три разные вещи, потому что «VNC не работает» бывает по трём разным причинам:
/// программы нет вовсе, программа есть но не запущена, запущена но слушает не там.
pub const DETECT_CMD: &str = concat!(
    "for b in x11vnc vncserver Xvnc tigervncserver x0vncserver; do ",
    "  p=$(command -v $b 2>/dev/null) && echo \"BIN:$b|$p\"; ",
    "done; ",
    // Кто слушает порты рабочего стола. 5900 - нулевой экран, дальше по одному на экран.
    "if command -v ss >/dev/null 2>&1; then ",
    "  ss -ltn 2>/dev/null | awk '$4 ~ /:59[0-9][0-9]$/ {print \"PORT:\" $4}'; ",
    "elif command -v netstat >/dev/null 2>&1; then ",
    "  netstat -ltn 2>/dev/null | awk '$4 ~ /:59[0-9][0-9]$/ {print \"PORT:\" $4}'; ",
    "elif [ -r /proc/net/tcp ]; then ",
    // Ни `ss`, ни `netstat` на минимальных системах может не быть вовсе - контейнер
    // стенда как раз такой. Тогда читаем ядро напрямую: `/proc/net/tcp` есть всегда.
    // Адрес и порт там шестнадцатеричные, поэтому отбираем по маске (170C..171F - это
    // 5900..5919, экраны с нулевого по девятнадцатый), а расшифровывает уже разбор.
    "  awk '$4 == \"0A\" && (toupper($2) ~ /:170[C-F]$/ || toupper($2) ~ /:171[0-9A-F]$/) ",
    "    {print \"PORTHEX:\" $2}' /proc/net/tcp 2>/dev/null; ",
    "fi; ",
    // Есть ли вообще графическая среда: без неё показывать будет нечего.
    "if [ -d /usr/share/xsessions ] && [ -n \"$(ls -A /usr/share/xsessions 2>/dev/null)\" ]; then ",
    "  echo 'DESK:есть'; else echo 'DESK:нет'; fi; ",
    // Чем ставить пакеты. От этого зависит вся дальнейшая команда установки.
    "for m in apt-get dnf yum apk zypper pacman; do ",
    "  command -v $m >/dev/null 2>&1 && { echo \"PM:$m\"; break; }; ",
    "done; ",
    // Может ли этот пользователь вообще стать root - иначе установка бессмысленна.
    "if sudo -n true 2>/dev/null; then echo 'SUDO:без пароля'; ",
    "elif command -v sudo >/dev/null 2>&1; then echo 'SUDO:по паролю'; ",
    "else echo 'SUDO:нет'; fi"
);

/// Разбирает ответ разведки.
pub fn parse_detect(stdout: &str) -> Value {
    let mut bins: Vec<Value> = Vec::new();
    let mut ports: Vec<String> = Vec::new();
    let mut desktop = String::new();
    let mut pm = String::new();
    let mut sudo = String::new();

    for line in stdout.lines() {
        let Some((tag, val)) = line.trim().split_once(':') else { continue };
        let val = val.trim();
        match tag {
            "BIN" => {
                let (name, path) = val.split_once('|').unwrap_or((val, ""));
                bins.push(json!({ "name": name, "path": path }));
            }
            "PORT" => {
                // `ss` печатает адрес вместе с портом; интересен порт, но и адрес важен:
                // слушающий только петлю - это правильно, а торчащий наружу - повод сказать.
                if !ports.contains(&val.to_string()) {
                    ports.push(val.to_string());
                }
            }
            // Запасной путь: ядро отдаёт адрес и порт шестнадцатеричными.
            "PORTHEX" => {
                if let Some(addr) = decode_proc_addr(val) {
                    if !ports.contains(&addr) {
                        ports.push(addr);
                    }
                }
            }
            "DESK" => desktop = val.to_string(),
            "PM" => pm = val.to_string(),
            "SUDO" => sudo = val.to_string(),
            _ => {}
        }
    }

    // Итог одной фразой: панели нужно не перечисление фактов, а ответ, что делать.
    let вывод = if !ports.is_empty() {
        "Рабочий стол уже слушает - можно подключаться"
    } else if !bins.is_empty() {
        "Программа установлена, но не запущена"
    } else if pm.is_empty() {
        "VNC не найден, и чем ставить пакеты - тоже неясно"
    } else {
        "VNC на сервере нет"
    };

    json!({
        "installed": bins,
        "listening": ports,
        "desktop": desktop,
        "packageManager": pm,
        "sudo": sudo,
        "summary": вывод,
        "canInstall": !pm.is_empty() && sudo != "нет",
    })
}

/// Расшифровывает адрес из `/proc/net/tcp`: `0100007F:170C` → `127.0.0.1:5900`.
///
/// Адрес там записан четырьмя байтами в обратном порядке - так его кладёт в память
/// машина, а ядро печатает как есть. Порт же обычный, просто шестнадцатеричный.
/// Не разобрали - возвращаем `None`: выдуманный адрес хуже отсутствующего.
fn decode_proc_addr(raw: &str) -> Option<String> {
    let (a, p) = raw.trim().split_once(':')?;
    let port = u16::from_str_radix(p, 16).ok()?;
    // IPv6 в этом файле записан тридцатью двумя знаками; для него точный разбор здесь
    // избыточен - достаточно сказать, что слушает, и на каком порту.
    if a.len() != 8 {
        return Some(format!("[::]:{port}"));
    }
    let n = u32::from_str_radix(a, 16).ok()?;
    // `to_le_bytes` уже разворачивает число в тот порядок, в каком байты лежали в памяти,
    // - а именно так их и печатает ядро. Второй разворот вернул бы 1.0.0.127 вместо
    // 127.0.0.1, что тест и поймал.
    let b = n.to_le_bytes();
    Some(format!("{}.{}.{}.{}:{port}", b[0], b[1], b[2], b[3]))
}

/// Имя пакета с сервером VNC для этого менеджера пакетов.
///
/// Названия у семейств разные, и промахнуться легко: `x11vnc` в Debian, `tigervnc-server`
/// в Fedora, и это не одна и та же программа, а две с похожей задачей.
pub fn package_for(pm: &str) -> Option<&'static str> {
    match pm {
        "apt-get" => Some("x11vnc"),
        "dnf" | "yum" => Some("tigervnc-server"),
        "apk" => Some("x11vnc"),
        "zypper" => Some("tigervnc"),
        "pacman" => Some("tigervnc"),
        _ => None,
    }
}

/// Команда установки под конкретный менеджер пакетов.
///
/// `sudo -S` читает пароль со стандартного ввода, `-p ''` убирает приглашение, чтобы оно
/// не смешалось с выводом. Всё неинтерактивно: спрашивать что-либо у команды, которая идёт
/// по каналу без терминала, некому.
pub fn install_cmd(pm: &str) -> Option<String> {
    let pkg = package_for(pm)?;
    let cmd = match pm {
        "apt-get" => format!("DEBIAN_FRONTEND=noninteractive apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq {pkg}"),
        "dnf" | "yum" => format!("{pm} install -y {pkg}"),
        "apk" => format!("apk add --no-cache {pkg}"),
        "zypper" => format!("zypper --non-interactive install {pkg}"),
        "pacman" => format!("pacman -S --noconfirm {pkg}"),
        _ => return None,
    };
    Some(format!("sudo -S -p '' sh -c '{cmd}' 2>&1"))
}

/// Проверка пароля рабочего стола.
///
/// Восемь символов - не наша придумка, а предел самого протокола: VNC шифрует пароль DES
/// с ключом такой длины и молча обрезает всё сверх того. Человек, задавший двенадцать,
/// потом не поймёт, почему подходит первая половина.
pub fn check_vnc_password(p: &str) -> Result<(), String> {
    if p.is_empty() {
        return Err("Пароль пустой".into());
    }
    if p.len() > 8 {
        return Err("VNC хранит только первые 8 символов пароля - задайте не длиннее".into());
    }
    // Набор символов узкий по двум причинам сразу. Первая: кириллица в пароле VNC не
    // работает - сервер ждёт однобайтовые символы. Вторая: на ветке `x11vnc -storepasswd`
    // пароль всё-таки становится доводом команды (стдина этот способ не умеет), и кавычка
    // оттуда сломала бы её целиком. Отказ, а не экранирование: восемь символов из этого
    // набора - не то ограничение, ради которого стоит рисковать.
    if !p
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "!#%*+,.:=?@_-".contains(c))
    {
        return Err(
            "Остальное сервер не примет: в пароле VNC можно латиницу, цифры и знаки !#%*+,.:=?@_-"
                .into(),
        );
    }
    Ok(())
}

/// Команда, задающая пароль рабочего стола. Сам пароль идёт на стандартный ввод.
///
/// Программ две, и они разные. У tigervnc есть `vncpasswd -f`: читает пароль со входа,
/// выдаёт зашифрованный на выход - пароль не появляется ни в строке команды, ни в списке
/// процессов. У x11vnc такого режима нет вовсе, там пароль обязан быть доводом, поэтому
/// эта ветка запасная. Раньше звали только x11vnc - на сервере с tigervnc это молча
/// не срабатывало, а `2>&1 >/dev/null` съедал объяснение, и панель показывала «не
/// удалось» без причины. Вывод больше не глушим: пусть программа сама скажет, что не так.
///
/// `umask 077` - чтобы файл пароля не оказался читаемым для всех на сервере.
pub const SET_PASSWORD_CMD: &str = concat!(
    "read -r p || exit 1; ",
    "umask 077; mkdir -p ~/.vnc || exit 1; ",
    "if command -v vncpasswd >/dev/null 2>&1; then ",
    "  echo \"$p\" | vncpasswd -f > ~/.vnc/passwd && echo OK; ",
    "elif command -v x11vnc >/dev/null 2>&1; then ",
    "  x11vnc -storepasswd \"$p\" ~/.vnc/passwd && echo OK; ",
    "else ",
    "  echo 'На сервере нет ни vncpasswd, ни x11vnc - задать пароль нечем' >&2; exit 1; ",
    "fi"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn три_разные_причины_различаются() {
        // «Не работает» бывает по-разному, и совет в каждом случае свой.
        let нет = parse_detect("PM:apt-get\nSUDO:по паролю\nDESK:есть\n");
        assert_eq!(нет["summary"], "VNC на сервере нет");
        assert_eq!(нет["canInstall"], true);

        let есть = parse_detect("BIN:x11vnc|/usr/bin/x11vnc\nPM:apt-get\nSUDO:по паролю\n");
        assert_eq!(есть["summary"], "Программа установлена, но не запущена");

        let слушает = parse_detect("BIN:x11vnc|/usr/bin/x11vnc\nPORT:127.0.0.1:5900\n");
        assert_eq!(слушает["summary"], "Рабочий стол уже слушает - можно подключаться");
    }

    #[test]
    fn пароль_умеет_обе_программы_и_не_лежит_в_команде() {
        // Прошлая версия знала только x11vnc и на сервере с tigervnc падала молча.
        assert!(SET_PASSWORD_CMD.contains("vncpasswd -f"));
        assert!(SET_PASSWORD_CMD.contains("x11vnc -storepasswd"));
        // Пароль приходит со входа, а не подставляется в текст команды.
        assert!(SET_PASSWORD_CMD.starts_with("read -r p"));
        // Внутрь литерала не должен попасть настоящий перевод строки: он ломает и
        // разбор команды на сервере, и сам литерал. Один раз уже попал.
        assert!(!SET_PASSWORD_CMD.contains('\n'));
        // Вывод самой записи пароля не глушится: иначе причина отказа пропадает по
        // дороге, и панель показывает «не удалось» без единого слова почему.
        assert!(!SET_PASSWORD_CMD.contains("vncpasswd -f > ~/.vnc/passwd 2>/dev/null"));
        assert!(SET_PASSWORD_CMD.contains(">&2"));
    }

    #[test]
    fn адрес_из_ядра_расшифровывается() {
        // `ss` и `netstat` есть не везде - контейнер стенда как раз без них. Тогда
        // единственный источник это `/proc/net/tcp`, где всё шестнадцатеричное.
        assert_eq!(decode_proc_addr("0100007F:170C").unwrap(), "127.0.0.1:5900");
        assert_eq!(decode_proc_addr("00000000:1717").unwrap(), "0.0.0.0:5911");
        // Шестнадцать байт - это IPv6; точный разбор тут избыточен.
        assert!(decode_proc_addr("00000000000000000000000001000000:170C")
            .unwrap()
            .contains("5900"));
        // Мусор не превращаем в адрес: выдуманный хуже отсутствующего.
        assert!(decode_proc_addr("не адрес").is_none());
        assert!(decode_proc_addr("").is_none());
    }

    #[test]
    fn запущенный_стол_виден_и_без_ss() {
        // Ровно случай контейнера стенда: программа есть, портов через `ss` не видно,
        // но ядро о них знает. Без этой ветки мы сказали бы «не запущена» про работающий.
        let v = parse_detect("BIN:x11vnc|/usr/bin/x11vnc\nPORTHEX:0100007F:170C\n");
        assert_eq!(v["summary"], "Рабочий стол уже слушает - можно подключаться");
        assert_eq!(v["listening"][0], "127.0.0.1:5900");
    }

    #[test]
    fn без_прав_ставить_нечего() {
        // Предлагать установку там, где её заведомо не выполнить, - обман.
        let v = parse_detect("PM:apt-get\nSUDO:нет\n");
        assert_eq!(v["canInstall"], false);
        let v = parse_detect("SUDO:по паролю\n");
        assert_eq!(v["canInstall"], false, "без менеджера пакетов ставить нечем");
    }

    #[test]
    fn пакет_зависит_от_семейства() {
        // В Debian и Fedora это разные программы с похожей задачей, а не одна.
        assert_eq!(package_for("apt-get"), Some("x11vnc"));
        assert_eq!(package_for("dnf"), Some("tigervnc-server"));
        assert_eq!(package_for("неизвестно"), None);
    }

    #[test]
    fn установка_идёт_неинтерактивно_и_с_паролем_на_вход() {
        let c = install_cmd("apt-get").unwrap();
        // Пароль в строке команды не появляется - он уйдёт на стандартный ввод.
        assert!(c.contains("sudo -S"), "{c}");
        assert!(!c.contains("password"), "{c}");
        // Спрашивать что-то у команды без терминала некому.
        assert!(c.contains("noninteractive"), "{c}");
        assert!(install_cmd("что-то своё").is_none());
    }

    #[test]
    fn длинный_пароль_отвергается_с_объяснением() {
        // Предел в восемь символов - свойство протокола, а не наша прихоть, и человеку
        // надо сказать почему, иначе он решит, что это блажь.
        assert!(check_vnc_password("secret12").is_ok());
        let e = check_vnc_password("оченьдлинныйпароль").unwrap_err();
        assert!(e.contains("8"), "{e}");
        assert!(check_vnc_password("").is_err());
        // Кириллица в пароле VNC не работает, и молча обрезать её нельзя.
        assert!(check_vnc_password("парол").is_err());
    }

    #[test]
    fn кавычка_в_пароле_не_доедет_до_команды() {
        // Через стандартный ввод пароль команду уже не сломает, но у x11vnc он остаётся
        // доводом - а заодно кириллицу и пробелы сам сервер VNC не принимает.
        assert!(check_vnc_password("a'b").is_err(), "кавычка прошла проверку");
        assert!(check_vnc_password("a\"b").is_err());
        assert!(check_vnc_password("a;b").is_err());
        assert!(check_vnc_password("a$b").is_err());
        assert!(check_vnc_password("a b").is_err(), "пробел тоже лишний");
        assert!(check_vnc_password("Pa55w0rd").is_ok());
    }
}
