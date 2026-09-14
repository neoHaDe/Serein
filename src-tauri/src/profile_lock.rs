//! Один профиль - один процесс.
//!
//! Профиль - это файлы, которые приложение переписывает целиком: серверы, секреты,
//! раскладка, известные ключи хостов. Каждая запись атомарна сама по себе, но два процесса
//! над одним профилем держат каждый свою копию в памяти и пишут поверх друг друга: сервер,
//! добавленный в одном окне, молча исчезает после сохранения в другом, а смена
//! мастер-пароля в одном процессе оставляет второй с секретами под прежним ключом.
//!
//! Защищаем профиль, а не приложение. Замок лежит в каталоге профиля и держится средствами
//! ОС на открытом файле: процесс упал - ОС сняла замок сама, и залипшего замка, который
//! пришлось бы удалять руками, не бывает. Два запуска с разными профилями
//! (`SEREIN_CONFIG_DIR`, портативный запуск) друг другу не мешают - и не должны.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const FILE: &str = "serein.lock";

/// Сколько ждём занятый профиль. Перезапуск после обновления стартует новый процесс раньше,
/// чем старый успевает выйти, и без ожидания обновлённое приложение отказывалось бы
/// запускаться ровно в момент обновления. Дольше не ждём: человек, открывший второе окно,
/// смотрит на пустой экран всё это время.
const WAIT: Duration = Duration::from_secs(2);

static HELD: OnceLock<File> = OnceLock::new();

/// Занимает профиль до конца процесса. `Err` - текст для человека: профиль занят.
pub fn hold(dir: &Path) -> Result<(), String> {
    match acquire(dir, WAIT) {
        Ok(Some(file)) => {
            let _ = HELD.set(file);
            Ok(())
        }
        Ok(None) => Err(busy_text(dir)),
        // Файловая система не умеет блокировок (так бывает на сетевых дисках) или файл не
        // создать. Не пустить из-за этого в приложение хуже, чем остаться без защиты от
        // редкого двойного запуска: профиль был бы недоступен вовсе.
        Err(e) => {
            eprintln!("профиль {}: замок недоступен, работаем без него: {e}", dir.display());
            Ok(())
        }
    }
}

/// `Ok(None)` - профиль занят и за отведённое время не освободился.
fn acquire(dir: &Path, wait: Duration) -> std::io::Result<Option<File>> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(FILE))?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => return Err(e),
        }
    }
}

fn busy_text(dir: &Path) -> String {
    format!(
        "Serein уже открыт с этим профилем:\n{}\n\nВторой экземпляр не запускается: два процесса переписывали бы серверы и пароли друг другу, и изменения одного молча пропадали бы. Переключитесь на открытое окно - оно может быть свёрнуто.",
        dir.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("serein-lock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn второй_захват_того_же_профиля_не_проходит() {
        // Замок ОС привязан к открытому файлу, а не к процессу, поэтому второй процесс
        // здесь честно изображает второй дескриптор того же файла.
        let dir = temp();
        let other = temp();
        let first = acquire(&dir, Duration::ZERO).unwrap().expect("первый захват");
        assert!(acquire(&dir, Duration::ZERO).unwrap().is_none(), "профиль уже занят");
        let neighbour = acquire(&other, Duration::ZERO).unwrap();
        assert!(neighbour.is_some(), "другой профиль свободен");
        drop(first);
        let again = acquire(&dir, Duration::ZERO).unwrap();
        assert!(again.is_some(), "после выхода первого профиль свободен");
        drop((again, neighbour));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(other);
    }

    #[test]
    fn перезапуск_дожидается_выхода_прежнего_процесса() {
        let dir = temp();
        let first = acquire(&dir, Duration::ZERO).unwrap().expect("первый захват");
        let old = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(first);
        });
        let second = acquire(&dir, Duration::from_secs(3)).unwrap();
        old.join().unwrap();
        assert!(second.is_some(), "новый процесс дождался старого");
        drop(second);
        let _ = std::fs::remove_dir_all(dir);
    }
}
