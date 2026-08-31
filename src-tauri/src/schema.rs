//! Версия схемы профиля и миграции между версиями.
//!
//! Формат бэкапа версионирован с самого начала (`backup.rs`), а сам профиль — нет. Это
//! разная ответственность: бэкап читают редко и осознанно, профиль — при каждом запуске.
//! Пока приложение стоит у одного человека, «просто поменяли формат» сходит с рук. Как
//! только обновление прилетает пользователям, любое изменение формы `servers.json` или
//! `settings.json` превращается в вопрос «что будет со старым профилем» — и отвечать на
//! него в момент, когда чужие данные уже сломаны, поздно.
//!
//! Поэтому здесь три вещи, и все три нужны до первой миграции, а не после:
//!
//! 1. **Номер схемы на диске** — чтобы вообще было с чем сравнивать.
//! 2. **Резервная копия перед миграцией** — чтобы неудачная миграция была обратимой.
//! 3. **Отказ при профиле из будущего** — если человек откатился на старую версию,
//!    молча дочитать профиль новее нашего значит потерять поля, которых мы не знаем.
//!
//! Версия 1 просто фиксирует нынешнюю форму: миграций пока нет. Смысл в том, чтобы первая
//! настоящая миграция писалась по готовому месту, а не в спешке поверх сломанного релиза.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Текущая версия схемы профиля. Поднимать вместе с добавлением миграции ниже.
pub const SCHEMA_VERSION: u32 = 1;

const FILE: &str = "schema.json";

fn schema_path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Версия схемы, записанная в профиле.
///
/// Профиль без файла считаем версией 1, а не нулём: такие профили создавались до появления
/// этого механизма и по форме соответствуют именно первой версии. Обратное — считать их
/// «нулевыми» и прогонять через миграции — сломало бы то, что и так в порядке.
pub fn stored_version(dir: &Path) -> u32 {
    std::fs::read_to_string(schema_path(dir))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("version").and_then(|n| n.as_u64()))
        .map(|n| n as u32)
        .unwrap_or(SCHEMA_VERSION)
}

fn write_version(dir: &Path, version: u32) -> Result<(), String> {
    let body = json!({
        "version": version,
        "app": env!("CARGO_PKG_VERSION"),
    });
    let text = serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?;
    std::fs::write(schema_path(dir), text).map_err(|e| e.to_string())
}

/// Копия профиля перед миграцией: `backups/pre-schema-<N>-<метка>/`.
///
/// Копируем только файлы верхнего уровня — весь профиль и есть плоский набор JSON. Логи и
/// прошлые копии не тащим: они не нужны для отката и раздули бы копию на порядки.
fn backup_before(dir: &Path, from: u32) -> Result<PathBuf, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = dir.join("backups").join(format!("pre-schema-{from}-{stamp}"));
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(name) = path.file_name() {
            std::fs::copy(&path, dest.join(name)).map_err(|e| e.to_string())?;
        }
    }
    Ok(dest)
}

/// Приводит профиль к текущей схеме.
///
/// Возвращает ошибку только там, где продолжать опасно: профиль из будущего или неудачная
/// миграция. Остальное — не повод не пускать человека в приложение.
pub fn migrate(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let from = stored_version(dir);

    if from > SCHEMA_VERSION {
        return Err(format!(
            "Профиль сохранён более новой версией Serein (схема {from}, эта версия понимает {SCHEMA_VERSION}). \
             Обновите приложение — иначе часть настроек потеряется при первой же записи."
        ));
    }
    if from == SCHEMA_VERSION {
        // Файла могло не быть — запишем, чтобы в следующий раз сравнивать было с чем.
        if !schema_path(dir).exists() {
            write_version(dir, SCHEMA_VERSION)?;
        }
        return Ok(());
    }

    let backup = backup_before(dir, from)?;
    let mut current = from;
    while current < SCHEMA_VERSION {
        current = step(dir, current).map_err(|e| {
            format!("Миграция профиля со схемы {current} не удалась: {e}. Копия до миграции: {}", backup.display())
        })?;
    }
    write_version(dir, SCHEMA_VERSION)
}

/// Один шаг миграции. Возвращает версию, к которой профиль приведён.
///
/// Пока таких шагов нет — версия 1 первая. Заготовка оставлена намеренно: когда формат
/// поменяется, здесь появится ветка, а вся обвязка (копия, порядок, отказ при откате)
/// уже будет проверена тестами.
fn step(_dir: &Path, from: u32) -> Result<u32, String> {
    match from {
        other => Err(format!("нет шага миграции со схемы {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("serein-schema-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("временный каталог");
        dir
    }

    #[test]
    fn profile_without_the_file_is_treated_as_current_not_as_zero() {
        // Профили, созданные до появления схемы, по форме и есть первая версия. Считать их
        // нулевыми значило бы прогнать через миграции то, что и так в порядке.
        let dir = tmp("legacy");
        assert_eq!(stored_version(&dir), SCHEMA_VERSION);
        migrate(&dir).expect("миграция");
        assert!(dir.join(FILE).exists(), "версия должна записаться на будущее");
        assert_eq!(stored_version(&dir), SCHEMA_VERSION);
    }

    #[test]
    fn profile_from_the_future_is_refused_with_an_explanation() {
        // Человек откатился на старую версию. Молча дочитать профиль новее нашего — значит
        // при первой же записи выбросить поля, которых мы не знаем.
        let dir = tmp("future");
        std::fs::write(dir.join(FILE), r#"{"version": 999}"#).unwrap();
        let err = migrate(&dir).expect_err("профиль из будущего нельзя принимать молча");
        assert!(err.contains("999"), "{err}");
        assert!(err.contains("новой версией"), "{err}");
    }

    #[test]
    fn migration_makes_a_backup_before_touching_anything() {
        // Проверяем именно обратимость: даже если шаг миграции провалится, данные до неё
        // должны остаться лежать рядом.
        let dir = tmp("backup");
        std::fs::write(dir.join(FILE), r#"{"version": 0}"#).unwrap();
        std::fs::write(dir.join("servers.json"), r#"[{"id":"важное"}]"#).unwrap();

        let err = migrate(&dir).expect_err("шага миграции с нулевой схемы нет");
        assert!(err.contains("Копия до миграции"), "{err}");

        let backups = std::fs::read_dir(dir.join("backups")).expect("каталог копий");
        let copy = backups.flatten().next().expect("копия").path();
        let saved = std::fs::read_to_string(copy.join("servers.json")).expect("сохранённые серверы");
        assert!(saved.contains("важное"), "копия должна содержать данные до миграции");
    }

    #[test]
    fn missing_profile_directory_is_not_an_error() {
        // Первый запуск: каталога ещё нет, и это нормальный путь, а не сбой.
        let dir = std::env::temp_dir().join("serein-schema-нет-такого-каталога");
        let _ = std::fs::remove_dir_all(&dir);
        migrate(&dir).expect("отсутствие профиля — не ошибка");
    }
}
