//! Опциональный мастер-пароль (порт vault.ts): scrypt-ключ доп. слоем поверх DPAPI.

use crate::{crypto, vaultkey};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::path::PathBuf;

const VERIFY_TOKEN: &str = "TERMINAL_VAULT_OK";

fn vault_path() -> PathBuf {
    crate::store::config_dir().join("vault.json")
}

fn read_config() -> Option<Value> {
    let txt = std::fs::read_to_string(vault_path()).ok()?;
    let cfg: Value = serde_json::from_str(&txt).ok()?;
    if cfg.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
        Some(cfg)
    } else {
        None
    }
}

pub fn is_enabled() -> bool {
    read_config().is_some()
}

pub fn status() -> Value {
    let enabled = is_enabled();
    json!({ "enabled": enabled, "locked": enabled && vaultkey::get().is_none() })
}

/// Параметры KDF из конфигурации хранилища.
///
/// Их не было в прежних сборках, поэтому отсутствие поля означает набор 2009 года.
/// Существующее хранилище на новые параметры молча не переводим: ключ изменился бы,
/// и все уже зашифрованные секреты перестали бы открываться. Новые параметры
/// достаются при следующем включении мастер-пароля — там секреты и так перешифровываются.
fn kdf_of(cfg: &Value) -> crypto::Kdf {
    let Some(k) = cfg.get("kdf") else {
        return crypto::KDF_LEGACY;
    };
    let num = |name: &str, d: u32| k.get(name).and_then(|v| v.as_u64()).unwrap_or(d as u64) as u32;
    crypto::Kdf {
        log_n: num("logN", crypto::KDF_LEGACY.log_n as u32) as u8,
        r: num("r", crypto::KDF_LEGACY.r),
        p: num("p", crypto::KDF_LEGACY.p),
    }
}

fn key_from(password: &str, salt_b64: &str, kdf: crypto::Kdf) -> Option<[u8; 32]> {
    let salt = STANDARD.decode(salt_b64).ok()?;
    Some(crypto::derive_key_with(password, &salt, kdf))
}

pub fn unlock(password: &str) -> bool {
    let Some(cfg) = read_config() else { return false };
    let salt = cfg.get("salt").and_then(|v| v.as_str()).unwrap_or("");
    let verifier = cfg.get("verifier").and_then(|v| v.as_str()).unwrap_or("");
    let Some(key) = key_from(password, salt, kdf_of(&cfg)) else { return false };
    match crypto::aes_decrypt(verifier, &key) {
        Ok(t) if t == VERIFY_TOKEN => {
            vaultkey::set(Some(key));
            true
        }
        _ => false,
    }
}

/// Минимальная длина мастер-пароля.
///
/// Двенадцать, а не «восемь плюс спецсимвол». Требования к составу — заглавные, цифры,
/// знаки — на практике дают `P@ssw0rd1`: формально проходит, подбирается мгновенно.
/// Длина работает лучше, и NIST SP 800-63B прямо рекомендует не навязывать состав, а
/// проверять по списку заведомо плохих. Здесь ровно это: длина плюс короткий список.
///
/// Ограничения сверху нет: длинная парольная фраза — это то, что мы хотим поощрять.
const MIN_MASTER_LEN: usize = 12;

/// Пароли, которые подбирают первыми. Список короткий намеренно: полный словарь утечек
/// сюда не поместится, а смысл — отсечь очевидное, не создавая иллюзии полной проверки.
const WORST: &[&str] = &[
    "password", "пароль", "123456789012", "qwertyuiop", "qwerty123456",
    "administrator", "changeme", "letmein12345", "iloveyou1234", "welcome12345",
];

/// Годится ли пароль в мастер-пароль.
///
/// Проверяем при включении, а не при разблокировке: у тех, кто уже включил мастер-пароль
/// раньше, ничего не должно перестать открываться из-за смены правил.
pub fn check_master_password(password: &str) -> Result<(), String> {
    // Считаем символы, а не байты: «пароль» — шесть символов и двенадцать байт, и по
    // байтам такая проверка прошла бы, чего мы точно не хотим.
    let chars = password.chars().count();
    if chars < MIN_MASTER_LEN {
        return Err(format!(
            "Мастер-паролем закрыты все сохранённые доступы, поэтому он должен быть длиннее: минимум {MIN_MASTER_LEN} символов (сейчас {chars}). Требований к спецсимволам нет — длинная фраза из нескольких слов подходит лучше короткой мешанины."
        ));
    }
    let lower = password.to_lowercase();
    if WORST.iter().any(|w| lower == *w) {
        return Err("Этот пароль есть в списке самых подбираемых. Возьмите фразу, которую не угадать.".into());
    }
    // Один символ на всю длину: формально длинно, по стойкости — ничто.
    let mut uniq: Vec<char> = password.chars().collect();
    uniq.sort_unstable();
    uniq.dedup();
    if uniq.len() < 4 {
        return Err("Слишком мало разных символов — такой пароль перебирается почти мгновенно.".into());
    }
    Ok(())
}

pub fn enable(password: &str) -> Value {
    if is_enabled() {
        return json!({ "ok": false, "error": "Мастер-пароль уже включён" });
    }
    if let Err(why) = check_master_password(password) {
        return json!({ "ok": false, "error": why });
    }
    // Вынимаем секреты при текущем (пустом) ключе, затем перешифровываем с мастер-слоем.
    let plain = crate::store::export_all_secrets();
    let salt = {
        use rand::RngCore;
        let mut s = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut s);
        s
    };
    let kdf = crypto::KDF_CURRENT;
    let key = crypto::derive_key_with(password, &salt, kdf);
    vaultkey::set(Some(key));
    if let Err(e) = crate::store::import_all_secrets(&plain) {
        return json!({ "ok": false, "error": e });
    }
    let verifier = match crypto::aes_encrypt(VERIFY_TOKEN, &key) {
        Ok(v) => v,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let cfg = json!({
        "enabled": true,
        "salt": STANDARD.encode(salt),
        "verifier": verifier,
        // Параметры записываем рядом: без них поднять стойкость в следующий раз
        // можно будет только сломав все существующие хранилища.
        "kdf": { "logN": kdf.log_n, "r": kdf.r, "p": kdf.p },
    });
    if let Err(e) = std::fs::write(vault_path(), serde_json::to_string_pretty(&cfg).unwrap()) {
        return json!({ "ok": false, "error": e.to_string() });
    }
    json!({ "ok": true })
}

pub fn disable(password: &str) -> Value {
    let Some(cfg) = read_config() else {
        return json!({ "ok": false, "error": "Мастер-пароль не задан" });
    };
    let salt = cfg.get("salt").and_then(|v| v.as_str()).unwrap_or("");
    let verifier = cfg.get("verifier").and_then(|v| v.as_str()).unwrap_or("");
    let Some(key) = key_from(password, salt, kdf_of(&cfg)) else {
        return json!({ "ok": false, "error": "Неверный пароль" });
    };
    if crypto::aes_decrypt(verifier, &key).ok().as_deref() != Some(VERIFY_TOKEN) {
        return json!({ "ok": false, "error": "Неверный пароль" });
    }
    vaultkey::set(Some(key));
    let plain = crate::store::export_all_secrets();
    vaultkey::set(None);
    if let Err(e) = crate::store::import_all_secrets(&plain) {
        return json!({ "ok": false, "error": e });
    }
    let _ = std::fs::remove_file(vault_path());
    json!({ "ok": true })
}

#[cfg(test)]
mod tests {
    use super::{check_master_password, MIN_MASTER_LEN};

    #[test]
    fn short_passwords_are_refused_with_the_reason() {
        // Прежний минимум — четыре символа — защищал все SSH-доступы примерно ничем.
        let err = check_master_password("qwerty").expect_err("шесть символов мало");
        assert!(err.contains(&MIN_MASTER_LEN.to_string()), "{err}");
        assert!(err.contains("6"), "в тексте должна быть текущая длина: {err}");
    }

    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        // «пароль» — шесть символов, но двенадцать байт. По байтам проверка прошла бы.
        assert!(check_master_password("пароль").is_err());
        // Одиннадцать кириллических символов — всё ещё мало, хотя байт заметно больше.
        assert!(check_master_password("паролькоро").is_err());
    }

    #[test]
    fn long_passphrase_passes_without_composition_rules() {
        // Ровно то, что мы хотим поощрять: длинная фраза без цифр и знаков.
        check_master_password("корова летит над амстердамом").expect("фраза должна проходить");
    }

    #[test]
    fn worst_known_passwords_are_refused_even_when_long_enough() {
        assert!(check_master_password("123456789012").is_err());
        assert!(check_master_password("QwErTyUiOp").is_err(), "регистр не должен помогать");
    }

    #[test]
    fn repeating_one_character_is_not_length() {
        assert!(check_master_password("ааааааааааааааааа").is_err());
        assert!(check_master_password("aaaaaaaaaaaaaaaa").is_err());
    }
}
