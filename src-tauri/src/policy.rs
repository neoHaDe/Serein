//! Политики администратора: настройки и запреты, которые пользователь на этой машине не меняет.
//!
//! Компания раздаёт Serein сотрудникам и хочет гарантий, а не просьб: журнал действий ведётся
//! и уходит в syslog, приложение не ходит в интернет, подключаться можно только к своим
//! серверам, пароли не хранятся на машине. Настройка в `settings.json` - это просьба:
//! пользователь правит файл сам. Политика берётся только оттуда, где её не подменить без прав
//! администратора:
//!
//! - на Windows ветка `HKLM\SOFTWARE\Policies\Serein`: отдельные значения, которые пишет шаблон
//!   групповых политик (`docs/policy/Serein.admx`), или одно значение `Policy` со строкой JSON;
//! - файл `%ProgramData%\Serein\policy.json` на Windows и `/etc/serein/policy.json` на Linux -
//!   но только если сам файл и его папка защищены от изменения. Место само по себе ничего не
//!   гарантирует: папку в `ProgramData` может создать обычный пользователь, и тогда он её
//!   владелец и может подменить положенный туда файл. Такой файл не применяется.
//!
//! Порядок: файл, затем JSON из реестра, затем отдельные значения реестра - следующий
//! перекрывает предыдущий по каждому ключу. Форма JSON:
//!
//! ```json
//! { "settings": { "actionLog": true, "offline": true },
//!   "allowedHosts": ["*.corp.local", "10.0.0.0/8"],
//!   "forbidLegacySshAlgorithms": true, "forbidSavedPasswords": true,
//!   "requireMasterPassword": true, "forbidLocalTerminal": true,
//!   "forbidSessionRecording": true }
//! ```
//!
//! Политика читается один раз при запуске: поменять её на ходу пользователю нечем, а
//! администратору достаточно перезапуска. Непонятная или незащищённая политика не применяется
//! вовсе, но это не тишина: ошибка видна в настройках и уходит в журнал действий при запуске.

use serde_json::{json, Map, Value};
use std::net::IpAddr;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Policy {
    /// Настройки, заданные политикой: значение и запрет менять.
    pub settings: Map<String, Value>,
    pub forbid_legacy_algorithms: bool,
    /// Куда можно подключаться. `None` - ограничения нет.
    pub allowed_hosts: Option<Vec<String>>,
    pub forbid_saved_passwords: bool,
    pub require_master_password: bool,
    pub forbid_local_terminal: bool,
    pub forbid_session_recording: bool,
    /// Откуда взята: пути и ключи реестра, которые что-то дали.
    pub sources: Vec<String>,
    pub error: Option<String>,
}

/// Источник политики: `Ok(None)` - его нет, `Err` - есть, но доверять ему нельзя.
pub type Source = (String, Result<Option<String>, String>);

/// Запреты-флажки: ключ JSON и поле политики.
const FLAGS: &[&str] = &[
    "forbidLegacySshAlgorithms",
    "forbidSavedPasswords",
    "requireMasterPassword",
    "forbidLocalTerminal",
    "forbidSessionRecording",
];

#[derive(Default)]
struct Parsed {
    settings: Map<String, Value>,
    flags: Vec<(String, bool)>,
    allowed_hosts: Option<Vec<String>>,
}

/// Шаблон адреса: имя целиком, `*.домен` (поддомены), `*` (всё) или подсеть `адрес/маска`.
fn validate_pattern(p: &str) -> Result<(), String> {
    let t = p.trim();
    if t.is_empty() {
        return Err("пустой адрес в «allowedHosts»".into());
    }
    if let Some((net, bits)) = t.split_once('/') {
        let net: IpAddr = net
            .parse()
            .map_err(|_| format!("«{t}»: подсеть должна начинаться с IP-адреса"))?;
        let max = if net.is_ipv4() { 32 } else { 128 };
        match bits.parse::<u8>() {
            Ok(b) if b <= max => return Ok(()),
            _ => return Err(format!("«{t}»: маска подсети от 0 до {max}")),
        }
    }
    let wildcard_ok = t == "*" || (t.starts_with("*.") && !t[2..].contains('*'));
    if t.contains('*') && !wildcard_ok {
        return Err(format!("«{t}»: звёздочка допустима только как «*» или «*.домен»"));
    }
    Ok(())
}

/// Разбор одного источника.
fn parse(text: &str) -> Result<Parsed, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("не JSON: {e}"))?;
    let obj = v.as_object().ok_or("ожидался объект JSON")?;
    let mut out = Parsed::default();
    for (key, val) in obj {
        match key.as_str() {
            "settings" => {
                out.settings = val.as_object().cloned().ok_or("«settings» должен быть объектом")?;
            }
            "allowedHosts" => {
                let list = val.as_array().ok_or("«allowedHosts» должен быть списком адресов")?;
                let mut hosts = Vec::with_capacity(list.len());
                for h in list {
                    let s = h.as_str().ok_or("в «allowedHosts» только строки")?;
                    validate_pattern(s)?;
                    hosts.push(s.trim().to_owned());
                }
                out.allowed_hosts = Some(hosts);
            }
            k if FLAGS.contains(&k) => {
                let b = val
                    .as_bool()
                    .ok_or_else(|| format!("«{k}» должен быть true или false"))?;
                out.flags.push((k.to_owned(), b));
            }
            other => {
                return Err(format!(
                    "неизвестный ключ «{other}» - опечатка в политике не должна молча ничего не запрещать"
                ));
            }
        }
    }
    Ok(out)
}

fn set_flag(p: &mut Policy, key: &str, value: bool) {
    match key {
        "forbidLegacySshAlgorithms" => p.forbid_legacy_algorithms = value,
        "forbidSavedPasswords" => p.forbid_saved_passwords = value,
        "requireMasterPassword" => p.require_master_password = value,
        "forbidLocalTerminal" => p.forbid_local_terminal = value,
        "forbidSessionRecording" => p.forbid_session_recording = value,
        _ => {}
    }
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
            Ok(one) => {
                p.settings.extend(one.settings);
                for (k, v) in one.flags {
                    set_flag(&mut p, &k, v);
                }
                if one.allowed_hosts.is_some() {
                    p.allowed_hosts = one.allowed_hosts;
                }
                p.sources.push(name.clone());
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    if !errors.is_empty() {
        // Наполовину применённая политика хуже неприменённой: неясно, что именно действует.
        return Policy {
            error: Some(errors.join("; ")),
            ..Policy::default()
        };
    }
    p
}

/// Значения реестра, которые пишет шаблон ADMX, - в ту же форму JSON, что и файл.
pub fn from_registry_values(
    dwords: &[(&str, u32)],
    strings: &[(&str, String)],
    multi: &[(&str, Vec<String>)],
) -> Option<String> {
    let mut obj = Map::new();
    let mut settings = Map::new();
    let dword = |name: &str| dwords.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
    let string = |name: &str| {
        strings
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.trim().to_owned())
    };
    if let Some(v) = dword("ActionLog") {
        settings.insert("actionLog".into(), json!(v != 0));
    }
    if let Some(v) = dword("Offline") {
        settings.insert("offline".into(), json!(v != 0));
    }
    if let Some(host) = string("SyslogHost").filter(|h| !h.is_empty()) {
        let protocol = if string("SyslogProtocol").as_deref() == Some("tcp") {
            "tcp"
        } else {
            "udp"
        };
        settings.insert(
            "actionLogSyslog".into(),
            json!({ "enabled": true, "host": host, "port": dword("SyslogPort").unwrap_or(514), "protocol": protocol }),
        );
    }
    for (value, key) in [
        ("ForbidLegacySshAlgorithms", "forbidLegacySshAlgorithms"),
        ("ForbidSavedPasswords", "forbidSavedPasswords"),
        ("RequireMasterPassword", "requireMasterPassword"),
        ("ForbidLocalTerminal", "forbidLocalTerminal"),
        ("ForbidSessionRecording", "forbidSessionRecording"),
    ] {
        if let Some(v) = dword(value) {
            obj.insert(key.into(), json!(v != 0));
        }
    }
    if let Some((_, hosts)) = multi.iter().find(|(n, _)| *n == "AllowedHosts") {
        let hosts: Vec<&str> = hosts.iter().map(|h| h.trim()).filter(|h| !h.is_empty()).collect();
        obj.insert("allowedHosts".into(), json!(hosts));
    }
    if !settings.is_empty() {
        obj.insert("settings".into(), Value::Object(settings));
    }
    (!obj.is_empty()).then(|| Value::Object(obj).to_string())
}

/// Путь к файлу политики.
///
/// На Windows каталог спрашивается у системы, а не у переменной окружения `ProgramData`:
/// переменную задаёт кто угодно в своём сеансе, и с подменённой политика администратора
/// просто «не находилась» - то есть снималась без единого следа. Отката на переменную нет
/// намеренно: не узнали путь - политика из файла не применяется, и об этом видно.
fn file_path() -> Result<std::path::PathBuf, String> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

        // SAFETY: GUID и флаг - константы самой системы, токен нулевой (текущий
        // пользователь). Строку система выделяет своим аллокатором, поэтому её
        // освобождает `CoTaskMemFree`, и делаем это сразу после копирования.
        let base = unsafe {
            let p = SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, HANDLE::default())
                .map_err(|e| format!("не узнать каталог ProgramData: {e}"))?;
            if p.is_null() {
                return Err("система не вернула каталог ProgramData".into());
            }
            let s = String::from_utf16_lossy(p.as_wide());
            CoTaskMemFree(Some(p.0 as *const _));
            s
        };
        if base.trim().is_empty() {
            return Err("система вернула пустой каталог ProgramData".into());
        }
        Ok(std::path::PathBuf::from(base).join("Serein").join("policy.json"))
    }
    #[cfg(not(windows))]
    {
        Ok(std::path::PathBuf::from("/etc/serein/policy.json"))
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
    let specific = if is_dir {
        FILE_DELETE_CHILD
    } else {
        FILE_WRITE_DATA | FILE_APPEND_DATA | GENERIC_WRITE
    };
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
    use windows::Win32::Security::{
        GetAce, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    };

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
        return Err(format!(
            "{}: права {:o} - писать может не только root",
            path.display(),
            m.mode() & 0o777
        ));
    }
    Ok(())
}

#[cfg(not(any(windows, unix)))]
fn check_object(path: &Path, _is_dir: bool) -> Result<(), String> {
    Err(format!(
        "{}: проверка прав на этой системе не поддерживается",
        path.display()
    ))
}

#[cfg(windows)]
const POLICY_KEY: &str = "SOFTWARE\\Policies\\Serein";

/// Строковое значение реестра: `Ok(None)` - его нет.
#[cfg(windows)]
fn reg_raw(name: &str, flags: windows::Win32::System::Registry::REG_ROUTINE_FLAGS) -> Result<Option<Vec<u16>>, String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE};
    let key = HSTRING::from(POLICY_KEY);
    let value = HSTRING::from(name);
    unsafe {
        let mut size: u32 = 0;
        let r = RegGetValueW(HKEY_LOCAL_MACHINE, &key, &value, flags, None, None, Some(&mut size));
        if r == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if r != ERROR_SUCCESS {
            return Err(format!("{name}: не прочитано (код {})", r.0));
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let r = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &key,
            &value,
            flags,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        );
        if r != ERROR_SUCCESS {
            return Err(format!("{name}: не прочитано (код {})", r.0));
        }
        buf.truncate((size as usize) / 2);
        Ok(Some(buf))
    }
}

#[cfg(windows)]
fn reg_string(name: &str) -> Result<Option<String>, String> {
    use windows::Win32::System::Registry::RRF_RT_REG_SZ;
    Ok(reg_raw(name, RRF_RT_REG_SZ)?.map(|buf| {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }))
}

#[cfg(windows)]
fn reg_dword(name: &str) -> Result<Option<u32>, String> {
    use windows::Win32::System::Registry::RRF_RT_REG_DWORD;
    Ok(reg_raw(name, RRF_RT_REG_DWORD)?
        .map(|buf| u32::from(buf.first().copied().unwrap_or(0)) | (u32::from(buf.get(1).copied().unwrap_or(0)) << 16)))
}

#[cfg(windows)]
fn reg_multi(name: &str) -> Result<Option<Vec<String>>, String> {
    use windows::Win32::System::Registry::RRF_RT_REG_MULTI_SZ;
    Ok(reg_raw(name, RRF_RT_REG_MULTI_SZ)?.map(|buf| {
        buf.split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }))
}

/// JSON целиком из значения `Policy`.
#[cfg(windows)]
fn registry_json() -> Result<Option<String>, String> {
    reg_string("Policy")
}

/// Отдельные значения шаблона ADMX.
#[cfg(windows)]
fn registry_values() -> Result<Option<String>, String> {
    let mut dwords = Vec::new();
    for name in [
        "ActionLog",
        "Offline",
        "SyslogPort",
        "ForbidLegacySshAlgorithms",
        "ForbidSavedPasswords",
        "RequireMasterPassword",
        "ForbidLocalTerminal",
        "ForbidSessionRecording",
    ] {
        if let Some(v) = reg_dword(name)? {
            dwords.push((name, v));
        }
    }
    let mut strings = Vec::new();
    for name in ["SyslogHost", "SyslogProtocol"] {
        if let Some(v) = reg_string(name)? {
            strings.push((name, v));
        }
    }
    let mut multi = Vec::new();
    if let Some(v) = reg_multi("AllowedHosts")? {
        multi.push(("AllowedHosts", v));
    }
    Ok(from_registry_values(&dwords, &strings, &multi))
}

#[cfg(not(windows))]
fn registry_json() -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(not(windows))]
fn registry_values() -> Result<Option<String>, String> {
    Ok(None)
}

fn load() -> Policy {
    // Не узнали путь - это не «файла нет»: иначе подмена окружения выглядела бы как
    // отсутствие политики. Источник объявляется с ошибкой, и она видна в настройках.
    let (name, text) = match file_path() {
        Ok(path) => (path.to_string_lossy().into_owned(), file_text(&path)),
        Err(e) => ("каталог ProgramData".to_owned(), Err(e)),
    };
    build(&[
        (name, text),
        ("HKLM\\SOFTWARE\\Policies\\Serein\\Policy".to_owned(), registry_json()),
        (
            "HKLM\\SOFTWARE\\Policies\\Serein (групповые политики)".to_owned(),
            registry_values(),
        ),
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

fn in_subnet(net: IpAddr, bits: u8, addr: IpAddr) -> bool {
    match (net, addr) {
        (IpAddr::V4(n), IpAddr::V4(a)) => {
            bits <= 32 && (bits == 0 || (u32::from(n) >> (32 - bits)) == (u32::from(a) >> (32 - bits)))
        }
        (IpAddr::V6(n), IpAddr::V6(a)) => {
            bits <= 128 && (bits == 0 || (u128::from(n) >> (128 - bits)) == (u128::from(a) >> (128 - bits)))
        }
        _ => false,
    }
}

fn pattern_matches(pattern: &str, host: &str) -> bool {
    let p = pattern.trim().to_ascii_lowercase();
    let h = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if p == "*" {
        return true;
    }
    if let Some(suffix) = p.strip_prefix("*.") {
        return h.len() > suffix.len() + 1 && h.ends_with(&format!(".{suffix}"));
    }
    if let Some((net, bits)) = p.split_once('/') {
        return match (net.parse::<IpAddr>(), bits.parse::<u8>(), h.parse::<IpAddr>()) {
            (Ok(n), Ok(b), Ok(a)) => in_subnet(n, b, a),
            _ => false,
        };
    }
    match (p.parse::<IpAddr>(), h.parse::<IpAddr>()) {
        (Ok(a), Ok(b)) => a == b,
        _ => p == h,
    }
}

pub fn host_allowed(patterns: &[String], host: &str) -> bool {
    patterns.iter().any(|p| pattern_matches(p, host))
}

/// Можно ли подключаться к этому адресу. Сравнивается адрес так, как он записан в профиле:
/// имя не разрешается в IP, поэтому подсеть совпадает только с адресом-IP.
pub fn check_host_with(p: &Policy, host: &str) -> Result<(), String> {
    match &p.allowed_hosts {
        Some(list) if !host_allowed(list, host) => Err(format!(
            "подключение к «{host}» запрещено политикой администратора: разрешены только {}",
            if list.is_empty() {
                "никакие адреса".to_owned()
            } else {
                list.join(", ")
            }
        )),
        _ => Ok(()),
    }
}

pub fn check_host(host: &str) -> Result<(), String> {
    check_host_with(current(), host)
}

/// Адрес назначения для всего, что открывает соединение мимо профиля сервера: туннели,
/// базы, рабочие столы, утилиты.
///
/// Раньше политика стояла только на самих SSH-подключениях, и запрещённый адрес спокойно
/// открывался туннелем через разрешённый сервер, запросом к базе или проверкой порта.
/// Отказ пишется в журнал действий: иначе о запрете знает только тот, кто нажал кнопку.
pub fn check_target(host: &str, what: &str) -> Result<(), String> {
    let r = check_host(host);
    if let Err(e) = &r {
        crate::actionlog::record(
            None,
            None,
            "policy.deny",
            json!({ "host": host, "what": what }),
            Err(e.clone()),
        );
    }
    r
}

pub fn forbids_legacy_algorithms() -> bool {
    current().forbid_legacy_algorithms
}

pub fn forbids_saved_passwords() -> bool {
    current().forbid_saved_passwords
}

pub fn requires_master_password() -> bool {
    current().require_master_password
}

pub fn forbids_local_terminal() -> bool {
    current().forbid_local_terminal
}

pub fn forbids_session_recording() -> bool {
    current().forbid_session_recording
}

pub fn status() -> Value {
    let p = current();
    json!({
        "sources": p.sources,
        "locked": p.settings.keys().collect::<Vec<_>>(),
        "allowedHosts": p.allowed_hosts,
        "forbidLegacySshAlgorithms": p.forbid_legacy_algorithms,
        "forbidSavedPasswords": p.forbid_saved_passwords,
        "requireMasterPassword": p.require_master_password,
        "forbidLocalTerminal": p.forbid_local_terminal,
        "forbidSessionRecording": p.forbid_session_recording,
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
    fn проверка_адреса_назначения_совпадает_с_проверкой_подключения() {
        // `check_target` - тот же список, только с записью отказа в журнал. Сам список
        // проверяется отдельно, здесь важно, что разрешение и запрет совпадают.
        let p = build(&[src("file", r#"{ "allowedHosts": ["10.0.0.0/8", "*.corp.local"] }"#)]);
        assert!(check_host_with(&p, "10.1.2.3").is_ok());
        assert!(check_host_with(&p, "db.corp.local").is_ok());
        let err = check_host_with(&p, "8.8.8.8").unwrap_err();
        assert!(err.contains("запрещено политикой"), "{err}");
        assert!(
            check_host_with(&p, "corp.local.evil.com").is_err(),
            "подстрока домена - не домен"
        );
    }

    #[test]
    fn путь_политики_не_зависит_от_переменной_окружения() {
        let before = file_path().expect("система обязана знать этот каталог");
        // SAFETY: одна переменная, читателей у неё в коде больше нет - путь берётся у
        // системы. Возвращаем прежнее значение сразу после проверки.
        let old = std::env::var_os("ProgramData");
        unsafe { std::env::set_var("ProgramData", "C:\\Users\\кто-угодно\\подмена") };
        let after = file_path().expect("путь берётся у системы");
        match old {
            Some(v) => unsafe { std::env::set_var("ProgramData", v) },
            None => unsafe { std::env::remove_var("ProgramData") },
        }
        assert_eq!(before, after, "подменённая ProgramData не должна уводить политику");
        assert!(
            !after.to_string_lossy().contains("подмена"),
            "путь политики взят из окружения: {}",
            after.display()
        );
    }

    #[test]
    fn политика_из_файла_и_реестра_реестр_перекрывает() {
        let p = build(&[
            src(
                "file",
                r#"{ "settings": { "actionLog": true, "offline": false }, "forbidLegacySshAlgorithms": true, "allowedHosts": ["a.local"] }"#,
            ),
            src(
                "reg",
                r#"{ "settings": { "offline": true }, "forbidLocalTerminal": true }"#,
            ),
            ("absent".to_owned(), Ok(None)),
        ]);
        assert_eq!(p.error, None);
        assert_eq!(p.settings.get("offline"), Some(&json!(true)), "реестр перекрыл файл");
        assert_eq!(p.settings.get("actionLog"), Some(&json!(true)));
        assert!(
            p.forbid_legacy_algorithms,
            "реестр не трогал этот ключ - остаётся из файла"
        );
        assert!(p.forbid_local_terminal);
        assert_eq!(p.allowed_hosts, Some(vec!["a.local".to_owned()]));
        assert_eq!(p.sources, vec!["file", "reg"]);
    }

    #[test]
    fn непонятная_или_недоверенная_политика_не_применяется_наполовину() {
        let p = build(&[
            src("file", r#"{ "settings": { "offline": true } }"#),
            src("reg", "{ oops"),
        ]);
        assert!(p.settings.is_empty() && !p.forbid_legacy_algorithms);
        assert!(
            p.error.as_deref().is_some_and(|e| e.starts_with("reg: не JSON")),
            "{:?}",
            p.error
        );
        let typo = build(&[src("file", r#"{ "forbidLegacySshAlgoritms": true }"#)]);
        assert!(
            typo.error.unwrap().contains("неизвестный ключ"),
            "опечатка не проходит молча"
        );
        assert!(build(&[src("f", r#"{ "forbidSavedPasswords": "yes" }"#)])
            .error
            .is_some());
        assert!(
            build(&[src("f", r#"{ "allowedHosts": ["10.0.0.0/33"] }"#)])
                .error
                .is_some(),
            "маска больше 32"
        );
        assert!(
            build(&[src("f", r#"{ "allowedHosts": ["db*.local"] }"#)])
                .error
                .is_some(),
            "звёздочка посередине"
        );
        let unsafe_file = build(&[("f".to_owned(), Err("не применён - владелец не администратор".into()))]);
        assert_eq!(
            unsafe_file.error.as_deref(),
            Some("f: не применён - владелец не администратор")
        );
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
    fn разрешённые_адреса_имя_поддомены_подсети() {
        let list: Vec<String> = [
            "prod.corp.local",
            "*.lab.local",
            "10.0.0.0/8",
            "fd00::/8",
            "192.168.1.5",
        ]
        .map(String::from)
        .to_vec();
        assert!(
            host_allowed(&list, "PROD.corp.local."),
            "регистр и точка на конце не важны"
        );
        assert!(host_allowed(&list, "db1.lab.local"));
        assert!(!host_allowed(&list, "lab.local"), "*.домен - только поддомены");
        assert!(!host_allowed(&list, "evil-lab.local"));
        assert!(host_allowed(&list, "10.20.30.40"));
        assert!(!host_allowed(&list, "11.0.0.1"));
        assert!(host_allowed(&list, "[fd12::1]"));
        assert!(host_allowed(&list, "192.168.1.5"));
        assert!(!host_allowed(&list, "prod.corp.local.evil.com"));
        assert!(host_allowed(&["*".to_owned()], "anything"));

        let p = build(&[src("f", r#"{ "allowedHosts": ["*.lab.local"] }"#)]);
        assert!(check_host_with(&p, "a.lab.local").is_ok());
        assert!(check_host_with(&p, "8.8.8.8")
            .unwrap_err()
            .contains("запрещено политикой"));
        assert!(
            check_host_with(&Policy::default(), "8.8.8.8").is_ok(),
            "без списка - без ограничения"
        );
        let none = build(&[src("f", r#"{ "allowedHosts": [] }"#)]);
        assert!(check_host_with(&none, "a").is_err(), "пустой список - никуда нельзя");
    }

    #[test]
    fn значения_групповых_политик_переводятся_в_политику() {
        let json = from_registry_values(
            &[
                ("ActionLog", 1),
                ("Offline", 0),
                ("SyslogPort", 6514),
                ("ForbidSavedPasswords", 1),
            ],
            &[("SyslogHost", " siem.corp ".into()), ("SyslogProtocol", "tcp".into())],
            &[("AllowedHosts", vec!["*.corp".into(), "".into()])],
        )
        .expect("значения есть");
        let p = build(&[src("reg", &json)]);
        assert_eq!(p.error, None);
        assert_eq!(p.settings.get("actionLog"), Some(&json!(true)));
        assert_eq!(p.settings.get("offline"), Some(&json!(false)));
        assert_eq!(
            p.settings.get("actionLogSyslog"),
            Some(&json!({ "enabled": true, "host": "siem.corp", "port": 6514, "protocol": "tcp" }))
        );
        assert!(p.forbid_saved_passwords && !p.require_master_password);
        assert_eq!(p.allowed_hosts, Some(vec!["*.corp".to_owned()]));
        assert_eq!(
            from_registry_values(&[], &[], &[]),
            None,
            "нет значений - нет источника"
        );
    }

    #[test]
    fn опасные_права_на_файл_и_папку() {
        assert!(grants_change(0x0002, false), "запись в файл");
        assert!(grants_change(0x0001_0000, false), "удаление файла");
        assert!(grants_change(0x0004_0000, true), "смена прав папки");
        assert!(grants_change(0x0040, true), "удаление файлов в папке");
        assert!(
            !grants_change(0x0002 | 0x0004, true),
            "создать новый файл в папке - не подмена существующего"
        );
        assert!(!grants_change(0x0012_0089, false), "чтение и исполнение");
        assert!(trusted_sid("S-1-5-32-544") && trusted_sid("S-1-5-18"));
        assert!(!trusted_sid("S-1-5-32-545"), "пользователи не доверены");
    }

    #[test]
    fn файл_созданный_обычным_пользователем_не_доверен() {
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
