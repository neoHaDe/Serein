//! Политики администратора: настройки, которые пользователь на этой машине не меняет.
//!
//! Компания раздаёт Serein сотрудникам и хочет гарантий, а не просьб: журнал действий ведётся
//! и уходит в syslog, приложение не ходит в интернет, старые алгоритмы SSH не включаются.
//! Настройка в `settings.json` - это просьба: пользователь правит файл сам. Политика берётся
//! только оттуда, где её не может подменить пользователь без прав администратора:
//!
//! - на Windows значение `Policy` (строка JSON) в `HKLM\SOFTWARE\Policies\Serein` - так её
//!   раздают групповыми политиками, и писать туда может только администратор;
//! - файл `%ProgramData%\Serein\policy.json` на Windows и `/etc/serein/policy.json` на Linux -
//!   но только если сам файл и его папка защищены от изменения. Место само по себе ничего не
//!   гарантирует: папку в `ProgramData` может создать обычный пользователь, и тогда он её
//!   владелец и может подменить положенный туда файл. Такой файл не применяется.
//!
//! Реестр перекрывает файл по каждому ключу. Форма:
//!
//! ```json
//! { "settings": { "actionLog": true, "offline": true },
//!   "forbidLegacySshAlgorithms": true }
//! ```
//!
//! Политика читается один раз при запуске: поменять её на ходу пользователю нечем, а
//! администратору достаточно перезапуска. Непонятная или незащищённая политика не применяется
//! вовсе, но это не тишина: ошибка видна в настройках и уходит в журнал действий при запуске.

use serde_json::{json, Map, Value};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Policy {
    /// Настройки, заданные политикой: значение и запрет менять.
    pub settings: Map<String, Value>,
    pub forbid_legacy_algorithms: bool,
    /// Откуда взята: пути и ключи реестра, которые что-то дали.
    pub sources: Vec<String>,
    pub error: Option<String>,
}

/// Источник политики: `Ok(None)` - его нет, `Err` - есть, но доверять ему нельзя.
pub type Source = (String, Result<Option<String>, String>);

/// Разбор одного источника.
fn parse(text: &str) -> Result<(Map<String, Value>, Option<bool>), String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("не JSON: {e}"))?;
    let obj = v.as_object().ok_or("ожидался объект JSON")?;
    for key in obj.keys() {
        if key != "settings" && key != "forbidLegacySshAlgorithms" {
            return Err(format!("неизвестный ключ «{key}» - опечатка в политике не должна молча ничего не запрещать"));
        }
    }
    let settings = match obj.get("settings") {
        None => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("«settings» должен быть объектом".into()),
    };
    let forbid = match obj.get("forbidLegacySshAlgorithms") {
        None => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err("«forbidLegacySshAlgorithms» должен быть true или false".into()),
    };
    Ok((settings, forbid))
}

/// Собирает политику из источников по порядку: следующий перекрывает предыдущий.
pub fn build(sources: &[Source]) -> Policy {
    let mut p = Policy::default();
    let mut errors = Vec::new();
    for (name, text) in sources {
        let parsed = match text {
            Ok(None) => continue,
            Ok(Some(t)) => parse(t),
            Err(e) => Err(e.clone()),
        };
        match parsed {
            Ok((settings, forbid)) => {
                p.settings.extend(settings);
                if let Some(f) = forbid {
                    p.forbid_legacy_algorithms = f;
                }
                p.sources.push(name.clone());
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    if !errors.is_empty() {
        // Наполовину применённая политика хуже неприменённой: неясно, что именно действует.
        return Policy { error: Some(errors.join("; ")), ..Policy::default() };
    }
    p
}

fn file_path() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
        std::path::PathBuf::from(base).join("Serein").join("policy.json")
    }
    #[cfg(not(windows))]
    {
        std::path::PathBuf::from("/etc/serein/policy.json")
    }
}

fn file_text(path: &Path) -> Result<Option<String>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        // Файл есть, но не читается: это не «политики нет».
        Err(e) => return Err(format!("не прочитан: {e}")),
    };
    protected(path).map_err(|e| format!("не применён - файл может изменить не только администратор: {e}"))?;
    Ok(Some(text))
}

/// SID, которым разрешено менять политику: администраторы, система, TrustedInstaller.
const TRUSTED_SIDS: &[&str] = &[
    "S-1-5-32-544",
    "S-1-5-18",
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
];

fn trusted_sid(sid: &str) -> bool {
    TRUSTED_SIDS.contains(&sid)
}

/// Даёт ли маска разрешения поменять файл политики - или, для папки, удалить его и положить
/// другой. Право создавать новые файлы в папке само по себе не опасно: существующий файл так
/// не заменить, а отсутствующий создатель сделает своим, и проверка владельца его отвергнет.
fn grants_change(mask: u32, is_dir: bool) -> bool {
    const DELETE: u32 = 0x0001_0000;
    const WRITE_DAC: u32 = 0x0004_0000;
    const WRITE_OWNER: u32 = 0x0008_0000;
    const GENERIC_ALL: u32 = 0x1000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_WRITE_DATA: u32 = 0x0002;
    const FILE_APPEND_DATA: u32 = 0x0004;
    const FILE_DELETE_CHILD: u32 = 0x0040;
    let common = DELETE | WRITE_DAC | WRITE_OWNER | GENERIC_ALL;
    let specific = if is_dir { FILE_DELETE_CHILD } else { FILE_WRITE_DATA | FILE_APPEND_DATA | GENERIC_WRITE };
    mask & (common | specific) != 0
}

/// Файл и его папка защищены от изменения не-администраторами.
fn protected(path: &Path) -> Result<(), String> {
    check_object(path, false)?;
    match path.parent() {
        Some(dir) => check_object(dir, true),
        None => Ok(()),
    }
}

#[cfg(windows)]
fn check_object(path: &Path, is_dir: bool) -> Result<(), String> {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL};
    use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows::Win32::Security::{GetAce, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID};

    unsafe fn sid_string(sid: PSID) -> Result<String, String> {
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(sid, &mut s).map_err(|e| e.to_string())?;
        let out = s.to_string().unwrap_or_default();
        let _ = LocalFree(HLOCAL(s.0.cast()));
        Ok(out)
    }

    let name = path.display().to_string();
    unsafe {
        let wide = HSTRING::from(path.as_os_str());
        let mut owner = PSID::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let r = GetNamedSecurityInfoW(
            &wide,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            &mut sd,
        );
        if r != ERROR_SUCCESS {
            return Err(format!("{name}: права не прочитать (код {})", r.0));
        }
        let verdict = (|| {
            let owner_sid = sid_string(owner)?;
            if !trusted_sid(&owner_sid) {
                return Err(format!("{name}: владелец {owner_sid}, а не администраторы или система"));
            }
            if dacl.is_null() {
                return Err(format!("{name}: нет списка доступа - изменить может кто угодно"));
            }
            for i in 0..u32::from((*dacl).AceCount) {
                let mut ace = std::ptr::null_mut();
                if GetAce(dacl, i, &mut ace).is_err() {
                    return Err(format!("{name}: список доступа не читается"));
                }
                let header = std::ptr::read_unaligned(ace as *const ACE_HEADER);
                // Запрещающие записи и записи только для наследования на сам объект не действуют.
                if header.AceType != 0 || header.AceFlags & 0x08 != 0 {
                    continue;
                }
                let mask = std::ptr::read_unaligned((ace as *const u8).add(4) as *const u32);
                let who = sid_string(PSID((ace as *mut u8).add(8).cast()))?;
                if grants_change(mask, is_dir) && !trusted_sid(&who) {
                    return Err(format!("{name}: изменить может {who}"));
                }
            }
            Ok(())
        })();
        let _ = LocalFree(HLOCAL(sd.0));
        verdict
    }
}

#[cfg(unix)]
fn check_object(path: &Path, _is_dir: bool) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;
    let m = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if m.uid() != 0 {
        return Err(format!("{}: владелец uid {}, а не root", path.display(), m.uid()));
    }
    if m.mode() & 0o022 != 0 {
        return Err(format!("{}: права {:o} - писать может не только root", path.display(), m.mode() & 0o777));
    }
    Ok(())
}

#[cfg(not(any(windows, unix)))]
fn check_object(path: &Path, _is_dir: bool) -> Result<(), String> {
    Err(format!("{}: проверка прав на этой системе не поддерживается", path.display()))
}

#[cfg(windows)]
fn registry_text() -> Result<Option<String>, String> {
    use windows::core::w;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    unsafe {
        let mut size: u32 = 0;
        let r = RegGetValueW(HKEY_LOCAL_MACHINE, w!("SOFTWARE\\Policies\\Serein"), w!("Policy"), RRF_RT_REG_SZ, None, None, Some(&mut size));
        if r == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if r != ERROR_SUCCESS {
            return Err(format!("не прочитан (код {})", r.0));
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let r = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Policies\\Serein"),
            w!("Policy"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        );
        if r != ERROR_SUCCESS {
            return Err(format!("не прочитан (код {})", r.0));
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(Some(String::from_utf16_lossy(&buf[..len])))
    }
}

#[cfg(not(windows))]
fn registry_text() -> Result<Option<String>, String> {
    Ok(None)
}

fn load() -> Policy {
    let path = file_path();
    build(&[
        (path.to_string_lossy().into_owned(), file_text(&path)),
        ("HKLM\\SOFTWARE\\Policies\\Serein\\Policy".to_owned(), registry_text()),
    ])
}

static POLICY: OnceLock<Policy> = OnceLock::new();

pub fn current() -> &'static Policy {
    POLICY.get_or_init(load)
}

/// Накладывает политику на настройки.
pub fn overlay_with(p: &Policy, settings: &mut Value) {
    if let Some(obj) = settings.as_object_mut() {
        for (k, v) in &p.settings {
            obj.insert(k.clone(), v.clone());
        }
    }
}

pub fn overlay(settings: &mut Value) {
    overlay_with(current(), settings);
}

/// Убирает из правки то, что задано политикой: в файл пользователя это не попадает вовсе.
pub fn strip_locked_with(p: &Policy, patch: &mut Value) {
    if let Some(obj) = patch.as_object_mut() {
        obj.retain(|k, _| !p.settings.contains_key(k));
    }
}

pub fn strip_locked(patch: &mut Value) {
    strip_locked_with(current(), patch);
}

pub fn forbids_legacy_algorithms() -> bool {
    current().forbid_legacy_algorithms
}

pub fn status() -> Value {
    let p = current();
    json!({
        "sources": p.sources,
        "locked": p.settings.keys().collect::<Vec<_>>(),
        "forbidLegacySshAlgorithms": p.forbid_legacy_algorithms,
        "error": p.error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(name: &str, text: &str) -> Source {
        (name.to_owned(), Ok(Some(text.to_owned())))
    }

    #[test]
    fn политика_из_файла_и_реестра_реестр_перекрывает() {
        let p = build(&[
            src("file", r#"{ "settings": { "actionLog": true, "offline": false }, "forbidLegacySshAlgorithms": true }"#),
            src("reg", r#"{ "settings": { "offline": true } }"#),
            ("absent".to_owned(), Ok(None)),
        ]);
        assert_eq!(p.error, None);
        assert_eq!(p.settings.get("offline"), Some(&json!(true)), "реестр перекрыл файл");
        assert_eq!(p.settings.get("actionLog"), Some(&json!(true)));
        assert!(p.forbid_legacy_algorithms, "реестр не трогал этот ключ - остаётся из файла");
        assert_eq!(p.sources, vec!["file", "reg"]);
    }

    #[test]
    fn непонятная_или_недоверенная_политика_не_применяется_наполовину() {
        let p = build(&[src("file", r#"{ "settings": { "offline": true } }"#), src("reg", "{ oops")]);
        assert!(p.settings.is_empty() && !p.forbid_legacy_algorithms);
        assert!(p.error.as_deref().is_some_and(|e| e.starts_with("reg: не JSON")), "{:?}", p.error);
        let typo = build(&[src("file", r#"{ "forbidLegacySshAlgoritms": true }"#)]);
        assert!(typo.error.unwrap().contains("неизвестный ключ"), "опечатка не проходит молча");
        assert!(build(&[src("f", r#"{ "forbidLegacySshAlgorithms": "yes" }"#)]).error.is_some());
        let unsafe_file = build(&[("f".to_owned(), Err("не применён - владелец не администратор".into()))]);
        assert_eq!(unsafe_file.error.as_deref(), Some("f: не применён - владелец не администратор"));
    }

    #[test]
    fn заданное_политикой_нельзя_поменять_и_оно_не_пишется_в_файл_пользователя() {
        let p = build(&[src("f", r#"{ "settings": { "actionLog": true } }"#)]);
        let mut settings = json!({ "actionLog": false, "theme": "x" });
        overlay_with(&p, &mut settings);
        assert_eq!(settings, json!({ "actionLog": true, "theme": "x" }));
        let mut patch = json!({ "actionLog": false, "theme": "y" });
        strip_locked_with(&p, &mut patch);
        assert_eq!(patch, json!({ "theme": "y" }));
    }

    #[test]
    fn опасные_права_на_файл_и_папку() {
        assert!(grants_change(0x0002, false), "запись в файл");
        assert!(grants_change(0x0001_0000, false), "удаление файла");
        assert!(grants_change(0x0004_0000, true), "смена прав папки");
        assert!(grants_change(0x0040, true), "удаление файлов в папке");
        assert!(!grants_change(0x0002 | 0x0004, true), "создать новый файл в папке - не подмена существующего");
        assert!(!grants_change(0x0012_0089, false), "чтение и исполнение");
        assert!(trusted_sid("S-1-5-32-544") && trusted_sid("S-1-5-18"));
        assert!(!trusted_sid("S-1-5-32-545"), "пользователи не доверены");
    }

    #[test]
    fn файл_созданный_обычным_пользователем_не_доверен() {
        // Именно так выглядит подложенная политика: файл и папка пользователя. На машине
        // разработчика и в CI процесс не пишет от имени администраторов-владельцев без прав
        // других на запись, поэтому проверка обязана отказать.
        let dir = std::env::temp_dir().join(format!("serein-policy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("policy.json");
        std::fs::write(&file, "{}").unwrap();
        let verdict = protected(&file);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(verdict.is_err(), "{verdict:?}");
    }

    #[cfg(windows)]
    #[test]
    fn системный_файл_под_защитой_администраторов_доверен() {
        let hosts = Path::new("C:\\Windows\\System32\\drivers\\etc\\hosts");
        if hosts.exists() {
            assert_eq!(protected(hosts), Ok(()));
        }
    }
}
