//! Простое JSON-хранилище в каталоге конфигурации (серверы/настройки/сниппеты/раскладка).
//! Значения храним как `serde_json::Value`, чтобы не мирроровать все типы фронтенда.

use crate::{crypto, os_secrets, vaultkey};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Имя переменной окружения, переопределяющей каталог профиля.
pub const CONFIG_DIR_ENV: &str = "SEREIN_CONFIG_DIR";

/// Каталог данных приложения. Старый `term-tauri` один раз переименовывается в `serein`.
///
/// `SEREIN_CONFIG_DIR` перекрывает выбор по умолчанию. Это нужно в трёх местах, и все
/// три настоящие: тесты, которым нельзя писать в профиль живого пользователя; портативный
/// запуск с флешки; закрытый контур, где профиль обязан лежать на заранее оговорённом
/// пути. Читаем один раз за процесс - иначе половина приложения работала бы с одним
/// каталогом, а половина с другим, если переменную поменяют на ходу.
pub fn config_dir() -> PathBuf {
    static OVERRIDE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    let chosen = OVERRIDE.get_or_init(|| {
        std::env::var_os(CONFIG_DIR_ENV)
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
    });
    if let Some(dir) = chosen {
        let _ = fs::create_dir_all(dir);
        static ONCE_OVERRIDE: std::sync::Once = std::sync::Once::new();
        ONCE_OVERRIDE.call_once(|| harden(dir));
        return dir.clone();
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let d = base.join("serein");
    let legacy = base.join("term-tauri");
    if !d.exists() && legacy.exists() {
        let _ = fs::rename(&legacy, &d);
    }
    let _ = fs::create_dir_all(&d);
    // Права выставляем один раз за запуск: `config_dir` зовётся на каждое чтение и запись,
    // и лишний системный вызов там ни к чему.
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| harden(&d));
    d
}

/// Закрыть каталог профиля от других пользователей машины.
///
/// `create_dir_all` создаёт каталог по umask - обычно это `755`, и на многопользовательской
/// Linux-машине `servers.json` с хостами, пользователями и `secrets.json` читает кто угодно.
/// На Windows этой дыры нет: `%APPDATA%` и так закрыт списком доступа, поэтому при переносе
/// проблема и не проявилась. Ведём себя как OpenSSH со своим `~/.ssh`.
#[cfg(unix)]
fn harden(d: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(d, fs::Permissions::from_mode(0o700));
    // Файлы, созданные прежними сборками, остались с правами от umask. Каталог `700`
    // уже закрывает к ним доступ, но оставлять `644` внутри - значит зависеть от того,
    // что каталог никто не откроет обратно.
    if let Ok(entries) = fs::read_dir(d) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() {
                let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o600));
            }
        }
    }
}

#[cfg(not(unix))]
fn harden(_d: &std::path::Path) {}

/// То же для файла: секреты и профили не должны быть доступны на чтение всем.
#[cfg(unix)]
pub(crate) fn restrict_file(p: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub(crate) fn restrict_file(_p: &std::path::Path) {}

fn dir() -> PathBuf {
    config_dir()
}

fn read_value(name: &str) -> Option<Value> {
    read_checked(name).ok().flatten()
}

/// Читает файл профиля, различая «его нет» и «его не прочитать».
///
/// Разница здесь стоит профиля целиком. Прежнее чтение на любую беду отвечало «ничего
/// нет»: повреждённый `servers.json` превращался в пустой список серверов, а первое же
/// сохранение записывало этот пустой список на место испорченного, но ещё поправимого
/// файла. Поэтому все записи теперь идут через эту проверку и при `Err` не пишут ничего.
fn read_checked(name: &str) -> Result<Option<Value>, String> {
    let path = dir().join(name);
    let txt = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("файл {name} не читается: {e}")),
    };
    serde_json::from_str(&txt).map(Some).map_err(|e| {
        format!("файл {name} повреждён ({e}) - он не перезаписан, разберитесь с ним сначала")
    })
}

/// Пишет файл профиля целиком или не пишет вовсе.
///
/// Прежняя запись шла прямо в целевой файл: прерванная на середине - полный диск,
/// выключение, снятие процесса - оставляла обрубок на месте профиля, и терялось всё.
/// Теперь содержимое сначала попадает в соседний временный файл, сбрасывается на диск, и
/// только потом одним переименованием занимает место прежнего. Переименование внутри
/// одного каталога заменяет файл целиком - и на Windows тоже.
fn write_value(name: &str, v: &Value) -> Result<(), String> {
    use std::io::Write as _;
    let txt = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    let path = dir().join(name);
    let tmp = dir().join(format!(".{name}.tmp-{}", Uuid::new_v4().simple()));
    let mut f = fs::File::create(&tmp).map_err(|e| format!("не создать временный файл: {e}"))?;
    // Права закрываем до записи содержимого: иначе секреты успевают полежать доступными.
    restrict_file(&tmp);
    let written = f
        .write_all(txt.as_bytes())
        .and_then(|_| f.flush())
        // Без этого переименование может обогнать сами данные: имя уже новое, а внутри
        // после выключения питания - пусто.
        .and_then(|_| f.sync_all());
    drop(f);
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(format!("не записать {name}: {e}"));
    }
    if let Err(e) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("не заменить {name}: {e}"));
    }
    restrict_file(&path);
    Ok(())
}

#[cfg(all(test, unix))]
mod perm_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Профиль хранит хосты, пользователей и секреты - читать его должен только владелец.
    /// На многопользовательской машине `755`/`644` от umask отдают всё это соседям.
    #[test]
    fn config_dir_and_files_are_private() {
        let d = config_dir();
        let mode = fs::metadata(&d).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "каталог профиля открыт: {mode:o}");

        write_value("perm-probe.json", &json!({ "x": 1 })).unwrap();
        let f = d.join("perm-probe.json");
        let fmode = fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        let _ = fs::remove_file(&f);
        assert_eq!(fmode, 0o600, "файл профиля открыт: {fmode:o}");
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;

    /// Своё имя файла на каждый тест: профиль здесь настоящий, соседей трогать нельзя.
    fn имя(tag: &str) -> String {
        format!("тест-{tag}-{}.json", Uuid::new_v4().simple())
    }

    #[test]
    fn повреждённый_файл_не_перезаписывается() {
        // Раньше нечитаемый файл выглядел как пустой список, и первое же сохранение
        // записывало этот пустой список на его место. Потерять профиль можно было одной
        // прерванной записью.
        let name = имя("битый");
        let path = dir().join(&name);
        fs::write(&path, "{ это не json").unwrap();

        let err = list_items_strict(&name).expect_err("битый файл обязан быть отказом");
        assert!(err.contains("повреждён"), "объяснение должно называть причину: {err}");
        assert!(
            upsert_item(&name, json!({ "id": "1" })).is_err(),
            "поверх нечитаемого файла не пишем"
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{ это не json",
            "файл обязан остаться как был"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn запись_не_оставляет_после_себя_временных_файлов() {
        let name = имя("целый");
        write_value(&name, &json!({ "a": 1 })).unwrap();
        assert_eq!(read_checked(&name).unwrap().unwrap()["a"], 1);

        let хвосты: Vec<String> = fs::read_dir(dir())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(&format!(".{name}.tmp-")))
            .collect();
        assert!(хвосты.is_empty(), "остались временные файлы: {хвосты:?}");
        let _ = fs::remove_file(dir().join(&name));
    }

    #[test]
    fn нет_файла_и_нечитаемый_файл_различаются() {
        // На этом различии держится вся защита: «нет» - это начать с чистого листа,
        // «не прочитать» - это остановиться и не трогать.
        assert!(read_checked(&имя("отсутствует")).unwrap().is_none());
    }
}

// ---------- Настройки ----------

fn default_settings() -> Value {
    json!({
        "theme": "Tokyo Night",
        "fontSize": 14,
        "fontFamily": "Cascadia Code, Consolas, \"Courier New\", monospace",
        "openLocalOnStart": false,
        "autoReconnect": false,
        "rdpNetworkProfile": "vpn",
        "rdpColorDepth": 16,
        "rdpEconomy": true,
        "rdpCaptureShortcuts": true,
        "sidebarWidth": 270,
        "sftpWidth": 380,
        "keybindings": {},
        "restoreTabsOnStart": false,
        "localShell": "auto",
        "density": "comfortable",
        "auxInTaskbar": false,
        "sftpConcurrency": 4,
        "restoreAuxOnStart": false,
        "sftpShowHidden": false,
        "sftpColOn": { "name": true, "ext": true, "mode": true, "size": true, "mtime": true },
        "sftpColWidths": { "name": 200, "ext": 64, "mode": 52, "size": 84, "mtime": 136 },
        "sftpSortCol": "name",
        "sftpSortDir": "asc",
        "externalEditor": ""
    })
}

pub fn settings_get() -> Value {
    let mut base = default_settings();
    if let (Some(b), Some(stored)) = (base.as_object_mut(), read_value("settings.json")) {
        if let Some(s) = stored.as_object() {
            for (k, v) in s {
                b.insert(k.clone(), v.clone());
            }
        }
    }
    base
}

pub fn settings_set(patch: Value) -> Result<Value, String> {
    let mut cur = settings_get();
    if let (Some(obj), Some(p)) = (cur.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            obj.insert(k.clone(), v.clone());
        }
        if let Some(n) = obj.get("sftpConcurrency").and_then(|v| v.as_u64()) {
            obj.insert("sftpConcurrency".into(), json!(n.clamp(1, 8)));
        }
    }
    write_value("settings.json", &cur)?;
    Ok(cur)
}

// ---------- Универсальный список объектов с полем id ----------

fn list_items(name: &str) -> Vec<Value> {
    read_value(name)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

/// То же, но для записи: непрочитанный файл - причина отказаться, а не начать с нуля.
fn list_items_strict(name: &str) -> Result<Vec<Value>, String> {
    Ok(read_checked(name)?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default())
}

fn upsert_item(name: &str, mut item: Value) -> Result<Value, String> {
    let id = item
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    if let Some(obj) = item.as_object_mut() {
        obj.insert("id".into(), Value::String(id.clone()));
    }
    let mut items = list_items_strict(name)?;
    if let Some(pos) = items
        .iter()
        .position(|i| i.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
    {
        items[pos] = item.clone();
    } else {
        items.push(item.clone());
    }
    write_value(name, &Value::Array(items))?;
    Ok(item)
}

fn delete_item(name: &str, id: &str) -> Result<(), String> {
    let items: Vec<Value> = list_items_strict(name)?
        .into_iter()
        .filter(|i| i.get("id").and_then(|v| v.as_str()) != Some(id))
        .collect();
    write_value(name, &Value::Array(items))
}

// ---------- Серверы ----------

// ----- Шифрование секретов (OS-слой + опциональный мастер-слой scrypt→AES) -----

fn os_protect(v: &str) -> Option<String> {
    os_secrets::protect(v)
}

/// Шифрует секрет для хранения.
///
/// `Ok(None)` - секрета нет вовсе (пустая строка). Отказ - именно отказ: раньше на его
/// месте был тот же `None`, и запертое хранилище системы или сбой шифрования выглядели
/// как «пароля не было». Сохранение при этом сообщало об успехе, а пароль исчезал.
fn encrypt_secret(value: &str) -> Result<Option<String>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    // Доп. слой мастер-пароля поверх OS-хранилища, если задан и разблокирован.
    let v = match vaultkey::get() {
        Some(mk) => format!(
            "mk:{}",
            crypto::aes_encrypt(value, &mk).map_err(|e| format!("не зашифровать секрет: {e}"))?
        ),
        None => value.to_string(),
    };
    os_protect(&v).map(Some).ok_or_else(|| {
        "хранилище секретов системы не приняло пароль - он не сохранён".to_owned()
    })
}

/// Расшифровывает секрет, различая «его нет» и «он недоступен».
///
/// Недоступен - это закрытая связка ключей или невведённый мастер-пароль. Считать это
/// отсутствием секрета нельзя: перешифровка и бэкап в таком случае молча теряли пароли.
fn read_secret_field(holder: &Value, field: &str) -> Result<Option<String>, String> {
    match holder.get(field).and_then(|v| v.as_str()) {
        None => Ok(None),
        Some(enc) => decrypt_secret(enc).map(Some).ok_or_else(|| {
            format!(
                "секрет «{field}» не расшифровывается: закрыто хранилище системы или не введён мастер-пароль"
            )
        }),
    }
}

/// Отпустить прежний секрет в OS-хранилище перед перезаписью или удалением.
/// На Windows это пустая операция, на Linux - удаление записи из связки ключей.
fn release_secret(holder: &Value, field: &str) {
    if let Some(s) = holder.get(field).and_then(|v| v.as_str()) {
        os_secrets::forget(s);
    }
}

fn decrypt_secret(enc: &str) -> Option<String> {
    // Префикс `plain:` больше не пишется (см. os_protect) - разбор оставлен только
    // для чтения профилей, сохранённых старыми сборками.
    let v = if let Some(rest) = enc.strip_prefix("plain:") {
        String::from_utf8(STANDARD.decode(rest).ok()?).ok()?
    } else {
        os_secrets::unprotect(enc)?
    };
    if let Some(rest) = v.strip_prefix("mk:") {
        let mk = vaultkey::get()?; // заблокировано - секрет недоступен
        crypto::aes_decrypt(rest, &mk).ok()
    } else {
        Some(v)
    }
}

fn read_secrets() -> Value {
    read_value("secrets.json").unwrap_or_else(|| json!({}))
}

/// Секреты для записи. Непрочитанный файл здесь опаснее всего: запись поверх него
/// стоила бы паролей ко всем остальным серверам сразу.
fn read_secrets_strict() -> Result<Value, String> {
    Ok(read_checked("secrets.json")?.unwrap_or_else(|| json!({})))
}

pub fn servers_list() -> Vec<Value> {
    list_items("servers.json")
}
/// Список серверов БЕЗ секретов - для UI.
pub fn servers_list_safe() -> Vec<Value> {
    servers_list()
        .into_iter()
        .map(|mut s| {
            if let Some(o) = s.as_object_mut() {
                o.remove("password");
                o.remove("passphrase");
            }
            s
        })
        .collect()
}

/// Перестановка серверов: меняет только группу и позицию, не трогая остальные поля.
///
/// Отдельная операция, а не цикл `servers_save`: тот прогоняет запись через слой секретов,
/// и перетаскивание мышью каждый раз перешифровывало бы пароли - лишний риск на ровном месте.
pub fn servers_reorder(items: &[Value]) -> Result<(), String> {
    let mut servers = list_items_strict("servers.json")?;
    for patch in items {
        let Some(id) = patch.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(target) = servers
            .iter_mut()
            .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(id))
        else {
            continue;
        };
        let Some(obj) = target.as_object_mut() else {
            continue;
        };
        match patch.get("group").and_then(|v| v.as_str()) {
            // Пустая строка = «без группы»: поле убираем, чтобы не плодить пустые ключи.
            Some(g) if !g.trim().is_empty() => {
                obj.insert("group".into(), json!(g.trim()));
            }
            _ => {
                obj.remove("group");
            }
        }
        if let Some(order) = patch.get("order").and_then(|v| v.as_u64()) {
            obj.insert("order".into(), json!(order));
        }
    }
    write_value("servers.json", &Value::Array(servers))
}

pub fn servers_save(mut cfg: Value) -> Result<Value, String> {
    let password = cfg.get("password").and_then(|v| v.as_str()).map(|s| s.to_string());
    let passphrase = cfg.get("passphrase").and_then(|v| v.as_str()).map(|s| s.to_string());
    let had_password = cfg.get("password").is_some();
    let had_passphrase = cfg.get("passphrase").is_some();
    if let Some(o) = cfg.as_object_mut() {
        o.remove("password");
        o.remove("passphrase");
    }
    // Номер выдаём до записи: секреты складываются под ним, а пишутся первыми.
    let id = match cfg.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        Some(i) => i.to_owned(),
        None => {
            let i = Uuid::new_v4().to_string();
            if let Some(o) = cfg.as_object_mut() {
                o.insert("id".into(), Value::String(i.clone()));
            }
            i
        }
    };

    // Порядок важен. Сначала шифруем - отказ на этом шаге не должен ничего менять.
    // Затем пишем секреты, и только потом сам сервер: осиротевший секрет безвреден, а
    // сервер без пароля выглядит как исправный и молча не подключается.
    let mut secrets = read_secrets_strict()?;
    let prev = secrets.get(&id).cloned().unwrap_or_else(|| json!({}));
    let mut next = Map::new();
    let pw = if had_password {
        password.as_deref().map(encrypt_secret).transpose()?.flatten().map(Value::String)
    } else {
        prev.get("password").cloned()
    };
    let pp = if had_passphrase {
        passphrase.as_deref().map(encrypt_secret).transpose()?.flatten().map(Value::String)
    } else {
        prev.get("passphrase").cloned()
    };
    if let Some(v) = pw.clone() {
        next.insert("password".into(), v);
    }
    if let Some(v) = pp.clone() {
        next.insert("passphrase".into(), v);
    }
    if let Some(o) = secrets.as_object_mut() {
        o.insert(id.clone(), Value::Object(next));
    }
    write_value("secrets.json", &secrets)?;
    // Прежнюю запись в хранилище системы отпускаем только теперь: до этого она была
    // единственной копией пароля, и отпустить её раньше значило остаться без него,
    // если что-то не запишется.
    if had_password && pw.is_some() {
        release_secret(&prev, "password");
    }
    if had_passphrase && pp.is_some() {
        release_secret(&prev, "passphrase");
    }
    upsert_item("servers.json", cfg)
}

pub fn servers_delete(id: &str) -> Result<(), String> {
    delete_item("servers.json", id)?;
    let mut secrets = read_secrets_strict()?;
    if let Some(sec) = secrets.get(id) {
        release_secret(sec, "password");
        release_secret(sec, "passphrase");
    }
    if let Some(o) = secrets.as_object_mut() {
        o.remove(id);
    }
    write_value("secrets.json", &secrets)
}

/// Полный конфиг сервера ВМЕСТЕ с расшифрованными секретами - только для подключения.
pub fn server_with_secrets(id: &str) -> Option<Value> {
    let mut base = servers_list()
        .into_iter()
        .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(id))?;
    let secrets = read_secrets();
    if let Some(sec) = secrets.get(id) {
        if let Some(o) = base.as_object_mut() {
            // Недоступный секрет здесь не ошибка: подключение просто спросит пароль само.
            for field in ["password", "passphrase"] {
                if let Ok(Some(p)) = read_secret_field(sec, field) {
                    o.insert(field.to_owned(), Value::String(p));
                }
            }
        }
    }
    Some(base)
}

/// Все серверы с расшифрованными секретами (для бэкапа).
pub fn list_servers_with_secrets() -> Vec<Value> {
    servers_list()
        .into_iter()
        .filter_map(|s| s.get("id").and_then(|v| v.as_str()).map(|id| id.to_string()))
        .filter_map(|id| server_with_secrets(&id))
        .collect()
}

/// Расшифровать все секреты (для re-wrap мастер-ключом).
///
/// Недоступный секрет - отказ на всю операцию. Иначе перешифровка записала бы вместо
/// него пустоту, и пароль исчезал бы в тот самый момент, когда человек меняет
/// мастер-пароль - то есть когда доверяет нам больше всего.
pub fn export_all_secrets() -> Result<Map<String, Value>, String> {
    let secrets = read_checked("secrets.json")?.unwrap_or_else(|| json!({}));
    let mut out = Map::new();
    if let Some(obj) = secrets.as_object() {
        for (id, sec) in obj {
            let pw = read_secret_field(sec, "password")?;
            let pp = read_secret_field(sec, "passphrase")?;
            out.insert(id.clone(), json!({ "password": pw, "passphrase": pp }));
        }
    }
    Ok(out)
}

/// Перешифровать секреты при ТЕКУЩЕМ состоянии ключа и записать.
pub fn import_all_secrets(map: &Map<String, Value>) -> Result<(), String> {
    let mut secrets = read_secrets_strict()?;
    // Сначала шифруем всё, и только потом пишем: отказ на середине не должен оставить
    // половину секретов под новым ключом, а половину под прежним.
    let mut ready: Vec<(String, Map<String, Value>)> = Vec::new();
    for (id, pair) in map {
        let mut next = Map::new();
        for field in ["password", "passphrase"] {
            let Some(plain) = pair.get(field).and_then(|v| v.as_str()) else {
                continue;
            };
            if let Some(enc) = encrypt_secret(plain)? {
                next.insert(field.to_owned(), Value::String(enc));
            }
        }
        ready.push((id.clone(), next));
    }
    if let Some(obj) = secrets.as_object_mut() {
        for (id, next) in ready {
            obj.insert(id, Value::Object(next));
        }
    }
    write_value("secrets.json", &secrets)
}


// ---------- Сниппеты ----------

pub fn snippets_list() -> Vec<Value> {
    list_items("snippets.json")
}
pub fn snippets_save(s: Value) -> Result<Value, String> {
    upsert_item("snippets.json", s)
}
pub fn snippets_delete(id: &str) -> Result<(), String> {
    delete_item("snippets.json", id)
}

// ---------- Раскладка вкладок ----------

pub fn layout_get() -> Value {
    read_value("layout.json").unwrap_or_else(|| Value::Array(vec![]))
}
pub fn layout_set(tabs: Value) -> Result<(), String> {
    write_value("layout.json", &tabs)
}

pub fn aux_layout_get() -> Value {
    read_value("aux-layout.json").unwrap_or_else(|| json!({ "windows": [] }))
}
pub fn aux_layout_set(layout: Value) -> Result<(), String> {
    write_value("aux-layout.json", &layout)
}

// Заглушка, чтобы избежать предупреждения о неиспользуемом импорте Map в некоторых конфигурациях.
#[allow(dead_code)]
fn _unused(_m: Map<String, Value>) {}
