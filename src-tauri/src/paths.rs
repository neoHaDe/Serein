//! Приведение путей из чужой системы к текущей.
//!
//! Профиль сервера хранит абсолютный путь к ключу — например
//! `C:\Users\hade\.ssh\id_ed25519`. После восстановления бэкапа на другой системе такого
//! файла нет, и подключение падало с голым `Нет такого файла или каталога (os error 2)`:
//! ни какого файла не хватает, ни что с этим делать.
//!
//! Здесь два уровня. `expand` разворачивает `~`, `$HOME` и `%USERPROFILE%` и приводит
//! разделители. `resolve_identity` идёт дальше: если файла по записанному пути нет,
//! пробует найти его по имени в домашнем `.ssh` — именно там ключ и оказывается после
//! ручного переноса.

use std::path::{Path, PathBuf};

/// Домашний каталог текущего пользователя.
fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// Разделители под текущую систему: на Unix обратный слэш в пути — это не разделитель,
/// а обычный символ имени, и `C:\Users\…` там превращается в один длинный файл.
fn to_native(p: &str) -> String {
    if cfg!(windows) {
        p.replace('/', "\\")
    } else {
        p.replace('\\', "/")
    }
}

/// Развернуть `~`, `$HOME`, `%USERPROFILE%` и привести разделители к текущей системе.
pub fn expand(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    let Some(home) = home_dir() else {
        return to_native(s);
    };
    let home = home.to_string_lossy().to_string();

    let expanded = if s == "~" {
        home
    } else if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        format!("{home}/{rest}")
    } else if let Some(rest) = s
        .strip_prefix("$HOME/")
        .or_else(|| s.strip_prefix("${HOME}/"))
    {
        format!("{home}/{rest}")
    } else if let Some(rest) = s
        .strip_prefix("%USERPROFILE%\\")
        .or_else(|| s.strip_prefix("%USERPROFILE%/"))
    {
        format!("{home}/{rest}")
    } else {
        s.to_string()
    };
    to_native(&expanded)
}

/// Хвост пути после каталога `.ssh`, если он там есть.
fn after_ssh_dir(p: &str) -> Option<&str> {
    let lower = p.to_ascii_lowercase();
    let idx = lower.rfind(".ssh/").or_else(|| lower.rfind(".ssh\\"))?;
    let rest = &p[idx + ".ssh/".len()..];
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

/// Имя файла без каталогов.
fn file_name_of(p: &str) -> Option<&str> {
    let cut = p.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0);
    let name = &p[cut..];
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Подобрать существующий файл ключа для пути, записанного на другой системе.
///
/// Возвращает первый путь, который реально существует; если не нашлось ничего —
/// развёрнутый исходный путь, чтобы в ошибке было что показать человеку.
/// Вынесено с параметрами ради тестов: логика не должна зависеть от того,
/// на какой системе и под каким пользователем идёт проверка.
pub fn resolve_identity_with(raw: &str, home: &Path, exists: &dyn Fn(&Path) -> bool) -> String {
    let direct = expand(raw);
    if direct.is_empty() || exists(Path::new(&direct)) {
        return direct;
    }

    let mut tries: Vec<PathBuf> = Vec::new();
    // Тот же относительный путь внутри домашнего `.ssh`: `…/.ssh/work/id_rsa` → `~/.ssh/work/id_rsa`.
    if let Some(rest) = after_ssh_dir(raw) {
        tries.push(home.join(".ssh").join(to_native(rest)));
    }
    // Просто по имени файла: после ручного переноса ключ обычно кладут прямо в `~/.ssh`.
    if let Some(name) = file_name_of(raw) {
        tries.push(home.join(".ssh").join(name));
    }

    for t in tries {
        if exists(&t) {
            return t.to_string_lossy().to_string();
        }
    }
    direct
}

/// То же, но по настоящей файловой системе и домашнему каталогу.
pub fn resolve_identity(raw: &str) -> String {
    match home_dir() {
        Some(h) => resolve_identity_with(raw, &h, &|p| p.exists()),
        None => expand(raw),
    }
}

/// Путь записан для другой системы и здесь заведомо не существует.
///
/// Нужен, чтобы отличить «файл удалили» от «профиль приехал с Windows»: советы разные.
pub fn looks_foreign(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty() {
        return false;
    }
    let windows_style = s.len() > 2
        && s.as_bytes()[1] == b':'
        && s.as_bytes()[0].is_ascii_alphabetic()
        && (s.as_bytes()[2] == b'\\' || s.as_bytes()[2] == b'/');
    if cfg!(windows) {
        // На Windows чужим выглядит юниксовый абсолютный путь.
        s.starts_with('/') && !windows_style
    } else {
        windows_style
    }
}

/// Человеческая ошибка вместо `os error 2`.
pub fn missing_key_error(raw: &str, resolved: &str) -> String {
    let mut msg = format!("Файл ключа не найден: {resolved}");
    if resolved != raw.trim() {
        msg.push_str(&format!(" (в профиле записан {})", raw.trim()));
    }
    if looks_foreign(raw) {
        msg.push_str(
            ". Путь записан для другой системы — так бывает после восстановления бэкапа: \
             скопируйте ключ в ~/.ssh и укажите путь в настройках сервера",
        );
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from(if cfg!(windows) { "C:\\Users\\me" } else { "/home/me" })
    }

    /// Проверяем без обращения к диску: «существует» задаём списком.
    fn exists_only(list: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p: &Path| {
            let s = p.to_string_lossy().replace('\\', "/");
            list.iter().any(|e| e.replace('\\', "/") == s)
        }
    }

    #[test]
    fn windows_key_path_falls_back_to_home_ssh() {
        // Ровно случай владельца: бэкап с Windows восстановили на Linux,
        // ключ лежит в ~/.ssh, а в профиле остался путь C:\Users\…
        let want = home().join(".ssh").join("id_ed25519");
        let got = resolve_identity_with(
            "C:\\Users\\hade\\.ssh\\id_ed25519",
            &home(),
            &exists_only(&["/home/me/.ssh/id_ed25519", "C:/Users/me/.ssh/id_ed25519"]),
        );
        assert_eq!(got, want.to_string_lossy());
    }

    #[test]
    fn nested_key_keeps_its_subfolder() {
        let want = home().join(".ssh").join("work").join("id_rsa");
        let got = resolve_identity_with(
            "C:\\Users\\hade\\.ssh\\work\\id_rsa",
            &home(),
            &exists_only(&["/home/me/.ssh/work/id_rsa", "C:/Users/me/.ssh/work/id_rsa"]),
        );
        assert_eq!(got, want.to_string_lossy());
    }

    #[test]
    fn existing_path_is_left_alone() {
        // Если файл на месте — ничего не подменяем, даже если имя совпадает с домашним.
        let real = home().join("keys").join("id_ed25519");
        let s = real.to_string_lossy().to_string();
        let leaked: &'static str = Box::leak(s.replace('\\', "/").into_boxed_str());
        let got = resolve_identity_with(&s, &home(), &exists_only(Box::leak(Box::new([leaked]))));
        assert_eq!(got, s);
    }

    #[test]
    fn unknown_key_returns_expanded_path_for_the_message() {
        let got = resolve_identity_with("~/.ssh/nope", &home(), &exists_only(&[]));
        assert!(got.ends_with("nope"), "{got}");
        assert!(!got.contains('~'), "тильда должна быть развёрнута: {got}");
    }

    #[test]
    fn foreign_paths_are_recognised() {
        if cfg!(windows) {
            assert!(looks_foreign("/home/hade/.ssh/id_ed25519"));
            assert!(!looks_foreign("C:\\Users\\hade\\.ssh\\id_ed25519"));
        } else {
            assert!(looks_foreign("C:\\Users\\hade\\.ssh\\id_ed25519"));
            assert!(looks_foreign("D:/keys/id_rsa"));
            assert!(!looks_foreign("/home/hade/.ssh/id_ed25519"));
        }
        assert!(!looks_foreign(""));
        assert!(!looks_foreign("~/.ssh/id_ed25519"));
    }

    #[test]
    fn message_names_the_file_and_explains_a_foreign_path() {
        let msg = missing_key_error("C:\\Users\\hade\\.ssh\\id_ed25519", "/home/me/.ssh/id_ed25519");
        assert!(msg.contains("/home/me/.ssh/id_ed25519"), "{msg}");
        if !cfg!(windows) {
            assert!(msg.contains("другой системы"), "{msg}");
        }
    }

    /// Тот же путь, но по настоящей файловой системе: чужой путь должен привести
    /// к файлу в домашнем `.ssh`. Пропускается, если каталога или файлов там нет.
    #[test]
    fn foreign_path_finds_a_real_file_in_home_ssh() {
        let Some(h) = home_dir() else { return };
        let dir = h.join(".ssh");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("пропуск: нет {}", dir.display());
            return;
        };
        let Some(name) = entries
            .flatten()
            .find(|e| e.path().is_file())
            .and_then(|e| e.file_name().into_string().ok())
        else {
            eprintln!("пропуск: в {} нет файлов", dir.display());
            return;
        };
        // Путь заведомо с другой системы: такого пользователя здесь нет.
        let got = resolve_identity(&format!("C:\\Users\\ktoto\\.ssh\\{name}"));
        assert_eq!(
            got,
            dir.join(&name).to_string_lossy(),
            "не нашли {name} в {}",
            dir.display()
        );
    }

    #[test]
    fn tilde_and_env_vars_expand() {
        let h = home_dir().map(|p| p.to_string_lossy().to_string());
        if let Some(h) = h {
            for raw in ["~/x", "$HOME/x", "%USERPROFILE%\\x"] {
                let got = expand(raw);
                assert!(got.starts_with(&h), "{raw} → {got}");
                assert!(got.ends_with('x'), "{raw} → {got}");
            }
        }
        assert_eq!(expand("  "), "");
    }
}
