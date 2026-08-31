//! Симметричное шифрование (порт crypto.ts): scrypt + AES-256-GCM.
//! Форматы пакетов СОВМЕСТИМЫ с Electron-версией (для общих бэкапов).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use scrypt::{scrypt, Params};

/// Параметры scrypt: `log2(N)`, `r`, `p`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Kdf {
    pub log_n: u8,
    pub r: u32,
    pub p: u32,
}

/// Набор 2009 года: им зашифровано всё, что создано до 2026-08-30. Читаем, но не пишем.
pub const KDF_LEGACY: Kdf = Kdf { log_n: 14, r: 8, p: 1 };

/// Текущий набор — рекомендация OWASP для scrypt.
///
/// N=2^17 это 128 МБ памяти на попытку против 16 МБ у прежнего: замер на рабочей машине
/// дал 154 мс вместо 19 мс. Для разблокировки раз в сессию разница незаметна, а перебор
/// украденного файла дорожает восьмикратно — а это единственное, ради чего KDF и нужен.
pub const KDF_CURRENT: Kdf = Kdf { log_n: 17, r: 8, p: 1 };

pub fn derive_key_with(password: &str, salt: &[u8], k: Kdf) -> Result<[u8; 32], String> {
    let mut out = [0u8; 32];
    // Мусорные параметры из чужого файла не должны ронять приложение — откатываемся
    // на прежний набор: хуже, чем хотелось, но лучше, чем паника в чужом коде.
    let params = Params::new(k.log_n, k.r, k.p, 32).or_else(|_| Params::new(14, 8, 1, 32))
        .map_err(|_| "Недопустимые параметры scrypt".to_string())?;
    scrypt(password.as_bytes(), salt, &params, &mut out)
        .map_err(|_| "Не удалось вывести ключ (scrypt)".to_string())?;
    Ok(out)
}

fn rand_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut v);
    v
}

/// base64( iv(12) | tag(16) | cipher ) ключом 32 байта.
pub fn aes_encrypt(plaintext: &str, key: &[u8; 32]) -> Result<String, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let iv = rand_bytes(12);
    let ct = cipher
        .encrypt(Nonce::from_slice(&iv), plaintext.as_bytes())
        .map_err(|_| "Шифрование не удалось".to_string())?;
    let (body, tag) = ct.split_at(ct.len() - 16);
    let mut out = Vec::with_capacity(28 + body.len());
    out.extend_from_slice(&iv);
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    Ok(STANDARD.encode(out))
}

pub fn aes_decrypt(packed: &str, key: &[u8; 32]) -> Result<String, String> {
    let buf = STANDARD.decode(packed).map_err(|e| e.to_string())?;
    if buf.len() < 28 {
        return Err("Повреждённый пакет".into());
    }
    let iv = &buf[0..12];
    let tag = &buf[12..28];
    let body = &buf[28..];
    let mut ct = Vec::with_capacity(body.len() + 16);
    ct.extend_from_slice(body);
    ct.extend_from_slice(tag);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let pt = cipher
        .decrypt(Nonce::from_slice(iv), ct.as_ref())
        .map_err(|_| "Неверный ключ или повреждение".to_string())?;
    String::from_utf8(pt).map_err(|e| e.to_string())
}

/// Метка формата 2: параметры KDF записаны внутрь пакета.
const V2_MAGIC: &[u8; 4] = b"SRN2";

/// base64( "SRN2" | logN | r | p | salt(16) | iv(12) | tag(16) | cipher ).
///
/// Параметры лежат в самом пакете: иначе поднять стойкость нельзя, не сломав каждый
/// уже созданный бэкап. Старый формат (без метки) по-прежнему читается.
pub fn encrypt_with_password(plaintext: &str, password: &str) -> Result<String, String> {
    let k = KDF_CURRENT;
    let salt = rand_bytes(16);
    let key = derive_key_with(password, &salt, k)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let iv = rand_bytes(12);
    let ct = cipher
        .encrypt(Nonce::from_slice(&iv), plaintext.as_bytes())
        .map_err(|_| "Шифрование не удалось".to_string())?;
    let (body, tag) = ct.split_at(ct.len() - 16);
    let mut out = Vec::new();
    out.extend_from_slice(V2_MAGIC);
    out.push(k.log_n);
    out.push(k.r as u8);
    out.push(k.p as u8);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&iv);
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    Ok(STANDARD.encode(out))
}

pub fn decrypt_with_password(packed: &str, password: &str) -> Result<String, String> {
    let buf = STANDARD.decode(packed.trim()).map_err(|e| e.to_string())?;
    // Формат 2 узнаём по метке; всё остальное — прежний формат без параметров.
    // Случайная соль могла бы начаться с тех же четырёх байт (шанс 1 к 4 миллиардам),
    // поэтому при неудаче формата 2 честно пробуем прежний, а не сдаёмся.
    let v2 = buf.starts_with(V2_MAGIC) && buf.len() >= 4 + 3 + 44;
    if v2 {
        let k = Kdf {
            log_n: buf[4],
            r: buf[5] as u32,
            p: buf[6] as u32,
        };
        if let Ok(txt) = open_packet(&buf[7..], password, k) {
            return Ok(txt);
        }
    }
    open_packet(&buf, password, KDF_LEGACY)
}

/// Разобрать `salt(16) | iv(12) | tag(16) | cipher` заданными параметрами KDF.
fn open_packet(buf: &[u8], password: &str, k: Kdf) -> Result<String, String> {
    if buf.len() < 44 {
        return Err("Повреждённый файл".into());
    }
    let salt = &buf[0..16];
    let iv = &buf[16..28];
    let tag = &buf[28..44];
    let body = &buf[44..];
    let key = derive_key_with(password, salt, k)?;
    let mut ct = Vec::new();
    ct.extend_from_slice(body);
    ct.extend_from_slice(tag);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let pt = cipher
        .decrypt(Nonce::from_slice(iv), ct.as_ref())
        .map_err(|_| "Неверный пароль или повреждённый файл".to_string())?;
    String::from_utf8(pt).map_err(|e| e.to_string())
}

// ── Тесты (QA-02, 2026-08-04) ───────────────────────────────────────────────
// Перенесены из TermiNAL/tests/crypto.test.ts: Tauri-порт переписал эту логику
// с TypeScript на Rust и остался без покрытия — то есть именно там, где
// вероятность ошибки максимальна, проверок не было. Ошибка здесь тихо
// компрометирует все сохранённые SSH-пароли и ключи.
#[cfg(test)]
mod tests {
    use super::*;

    /// Быстрый набор параметров для проверок, где важна не стойкость,
    /// а само поведение вывода ключа. Прогонять их на боевых 2^17 — держать
    /// набор тестов медленным без всякой пользы.
    fn key(pw: &str, salt: &[u8]) -> [u8; 32] {
        derive_key_with(pw, salt, KDF_LEGACY).expect("scrypt в тесте")
    }

    /// Пакеты, созданные прежними сборками, обязаны читаться и после подъёма стойкости.
    /// Иначе смена параметров молча превращает все бэкапы пользователя в мусор.
    #[test]
    fn old_format_packets_still_open() {
        // Собираем пакет ровно так, как это делала прежняя версия: без метки и параметров.
        let secret = "секрет из старого бэкапа";
        let salt = rand_bytes(16);
        let key = derive_key_with("pass", &salt, KDF_LEGACY).expect("scrypt в тесте");
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let iv = rand_bytes(12);
        let ct = cipher.encrypt(Nonce::from_slice(&iv), secret.as_bytes()).unwrap();
        let (body, tag) = ct.split_at(ct.len() - 16);
        let mut out = Vec::new();
        out.extend_from_slice(&salt);
        out.extend_from_slice(&iv);
        out.extend_from_slice(tag);
        out.extend_from_slice(body);
        let packed = STANDARD.encode(out);

        assert_eq!(decrypt_with_password(&packed, "pass").unwrap(), secret);
        assert!(decrypt_with_password(&packed, "wrong").is_err());
    }

    /// Новый пакет должен нести метку и текущие параметры — иначе подъём стойкости
    /// не состоялся бы, а тест round-trip этого не заметил бы.
    #[test]
    fn new_packets_carry_current_kdf() {
        let packed = encrypt_with_password("x", "pass").unwrap();
        let buf = STANDARD.decode(&packed).unwrap();
        assert_eq!(&buf[0..4], V2_MAGIC, "нет метки формата");
        assert_eq!(buf[4], KDF_CURRENT.log_n, "записан не текущий log2(N)");
        assert!(KDF_CURRENT.log_n > KDF_LEGACY.log_n, "новый набор должен быть строже");
    }

    #[test]
    fn encrypt_with_password_round_trip() {
        let secret = "пароль-сервера-42 🔐";
        let packed = encrypt_with_password(secret, "master-pass").expect("шифрование");
        let back = decrypt_with_password(&packed, "master-pass").expect("расшифровка");
        assert_eq!(back, secret);
    }

    #[test]
    fn decrypt_with_wrong_password_fails() {
        let packed = encrypt_with_password("data", "right").expect("шифрование");
        assert!(
            decrypt_with_password(&packed, "wrong").is_err(),
            "неверный пароль обязан давать ошибку, а не мусор"
        );
    }

    #[test]
    fn each_encryption_is_unique() {
        // Случайные соль и IV: одинаковый вход не должен давать одинаковый пакет,
        // иначе по совпадению шифртекстов видно, что пароли совпадают.
        let a = encrypt_with_password("x", "p").expect("шифрование");
        let b = encrypt_with_password("x", "p").expect("шифрование");
        assert_ne!(a, b);
    }

    #[test]
    fn aes_round_trip_with_explicit_key() {
        let key = key("pw", &[7u8; 16]);
        let packed = aes_encrypt("hello", &key).expect("шифрование");
        assert_eq!(aes_decrypt(&packed, &key).expect("расшифровка"), "hello");
    }

    #[test]
    fn aes_decrypt_with_other_key_fails() {
        let k1 = key("pw", &[1u8; 16]);
        let k2 = key("pw", &[2u8; 16]);
        let packed = aes_encrypt("hello", &k1).expect("шифрование");
        assert!(aes_decrypt(&packed, &k2).is_err());
    }

    #[test]
    fn derive_key_is_deterministic_and_salt_sensitive() {
        // Та же пара (пароль, соль) — тот же ключ, иначе хранилище не откроется
        // после перезапуска. Разная соль — разный ключ, иначе соль бесполезна.
        assert_eq!(key("pw", &[9u8; 16]), key("pw", &[9u8; 16]));
        assert_ne!(key("pw", &[9u8; 16]), key("pw", &[8u8; 16]));
        assert_ne!(key("pw1", &[9u8; 16]), key("pw2", &[9u8; 16]));
    }

    #[test]
    fn corrupted_package_fails_gracefully() {
        // Битый файл хранилища должен давать Err, а не панику: паника в Tauri
        // роняет команду целиком и пользователь видит пустое окно без объяснения.
        let key = key("pw", &[3u8; 16]);
        for bad in ["", "не-base64!!", "AAAA", "***"] {
            assert!(aes_decrypt(bad, &key).is_err(), "должно быть Err на {:?}", bad);
        }
    }

    #[test]
    fn empty_plaintext_survives_round_trip() {
        let packed = encrypt_with_password("", "master").expect("шифрование");
        assert_eq!(decrypt_with_password(&packed, "master").expect("расшифровка"), "");
    }
}
