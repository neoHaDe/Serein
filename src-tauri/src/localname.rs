//! Имена, пришедшие с сервера, - в путь на своей машине.
//!
//! Имя файла из ответа сервера - это не строка, которой можно доверять. На Linux имя
//! `..\..\выход.txt` совершенно законно: обратная косая там обычный символ. Windows же
//! читает её как разделитель каталогов, и такое имя, склеенное с папкой скачивания,
//! указывает уже за её пределы. Так же работают `C:\что-то`, `\сервер\доля` и
//! `файл:поток` - последний пишет в скрытый поток NTFS.
//!
//! Поэтому каждое имя проверяется как **один** элемент пути целевой системы, а сомнительные
//! отклоняются с объяснением. Не переименовываются молча: полученный файл должен зваться
//! так же, как на сервере, а если это невозможно - человек должен об этом знать.
//!
//! Ссылки мы не разыменовываем и не создаём: скачивание только пишет обычные файлы и
//! создаёт каталоги под выбранным корнем. Если сам этот корень - ссылка, ведущая куда-то
//! ещё, это выбор хозяина машины, а не действие сервера.

use std::path::{Component, Path, PathBuf};

/// Устройства DOS: имя с таким началом Windows открывает как устройство, а не файл.
///
/// Проверяется часть до первой точки: `NUL.txt` - то же устройство, что и `NUL`.
const DOS_DEVICES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Проверяет имя как один элемент локального пути.
///
/// Возвращает то же имя, если оно безопасно, и объяснение отказа, если нет.
pub fn safe_component(name: &str) -> Result<&str, String> {
    let bad = |why: &str| Err(format!("имя «{name}» {why}"));

    if name.is_empty() {
        return Err("пустое имя файла".into());
    }
    if name == "." || name == ".." {
        return bad("указывает на каталог, а не на файл");
    }
    if name.contains('/') {
        return bad("содержит разделитель каталогов");
    }
    if name.chars().any(|c| c == '\0' || c.is_control()) {
        return bad("содержит управляющий символ");
    }
    if cfg!(windows) {
        // На Windows это не придирки: такие имена система либо считает путём, либо не
        // создаёт вовсе, либо тихо превращает в другое.
        if name.contains('\\') {
            return bad("содержит обратную косую - на Windows это разделитель каталогов");
        }
        if name.contains(':') {
            return bad("содержит двоеточие - на Windows это диск или поток NTFS");
        }
        if let Some(c) = name.chars().find(|c| "*?\"<>|".contains(*c)) {
            return bad(&format!("содержит недопустимый на Windows символ «{c}»"));
        }
        // Windows молча отбрасывает точки и пробелы в конце, и `отчёт.txt.` попадает в
        // тот же файл, что `отчёт.txt`.
        if name.ends_with('.') || name.ends_with(' ') {
            return bad("заканчивается точкой или пробелом - Windows их отбросит");
        }
        let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
        if DOS_DEVICES.contains(&stem.as_str()) {
            return bad("совпадает с именем устройства DOS");
        }
    }
    Ok(name)
}

/// Приводит путь к виду без `.` и `..`, не обращаясь к диску.
///
/// Именно без обращения: путь ещё не существует - он и создаётся этой операцией.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Останется ли путь внутри выбранного корня.
///
/// Вторая сеть после проверки имён: сами имена уже проверены, но путь собирается в
/// нескольких местах, и ошибка в любом из них не должна доходить до записи файла.
pub fn under_root(root: &Path, path: &Path) -> bool {
    let (root, path) = (normalize(root), normalize(path));
    path.starts_with(&root) && path != root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn имя_с_обратной_косой_не_проходит_на_windows() {
        // То самое имя, из-за которого файл оказывался за пределами папки скачивания.
        let r = safe_component(r"..\..\выход.txt");
        if cfg!(windows) {
            assert!(r.is_err(), "на Windows это путь, а не имя");
        } else {
            assert!(r.is_ok(), "на юниксах обратная косая - обычный символ имени");
        }
    }

    #[test]
    fn обычные_имена_включая_кириллицу_проходят() {
        for name in ["отчёт.txt", "file name.tar.gz", "два--дефиса", ".bashrc", "日本語.md"] {
            assert_eq!(safe_component(name).unwrap(), name, "имя «{name}» безопасно");
        }
    }

    #[test]
    fn путь_вместо_имени_отклоняется_везде() {
        for name in ["", ".", "..", "a/b", "/etc/passwd", "x\u{0}y", "стр\nока"] {
            assert!(safe_component(name).is_err(), "имя «{name}» должно быть отклонено");
        }
    }

    #[cfg(windows)]
    #[test]
    fn ловушки_windows_отклоняются() {
        for name in ["C:evil", "файл:поток", "NUL", "nul.txt", "отчёт.txt.", "имя ", "a*b", "a?b"] {
            assert!(safe_component(name).is_err(), "имя «{name}» опасно на Windows");
        }
    }

    #[test]
    fn выход_из_корня_виден_и_после_склейки() {
        let root = Path::new("/home/u/Загрузки");
        assert!(under_root(root, Path::new("/home/u/Загрузки/папка/файл")));
        assert!(!under_root(root, Path::new("/home/u/Загрузки/../тайное")));
        assert!(!under_root(root, Path::new("/home/u/Загрузки")), "сам корень - не файл в нём");
        assert!(!under_root(root, Path::new("/etc/passwd")));
    }
}
