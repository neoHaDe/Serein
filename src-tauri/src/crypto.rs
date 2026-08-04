//! Симметричное шифрование (порт crypto.ts): scrypt + AES-256-GCM.
//! Форматы пакетов СОВМЕСТИМЫ с Electron-версией (для общих бэкапов).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use scrypt::{scrypt, Params};

/// scrypt(N=16384, r=8, p=1) → 32-байтный ключ.
pub fn derive_key(password: &str, salt: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let params = Params::new(14, 8, 1, 32).expect("scrypt params");
    scrypt(password.as_bytes(), salt, &params, &mut out).expect("scrypt");
    out
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

/// base64( salt(16) | iv(12) | tag(16) | cipher ) — шифрование паролем.
pub fn encrypt_with_password(plaintext: &str, password: &str) -> Result<String, String> {
    let salt = rand_bytes(16);
    let key = derive_key(password, &salt);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let iv = rand_bytes(12);
    let ct = cipher
        .encrypt(Nonce::from_slice(&iv), plaintext.as_bytes())
        .map_err(|_| "Шифрование не удалось".to_string())?;
    let (body, tag) = ct.split_at(ct.len() - 16);
    let mut out = Vec::new();
    out.extend_from_slice(&salt);
    out.extend_from_slice(&iv);
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    Ok(STANDARD.encode(out))
}

pub fn decrypt_with_password(packed: &str, password: &str) -> Result<String, String> {
    let buf = STANDARD.decode(packed.trim()).map_err(|e| e.to_string())?;
    if buf.len() < 44 {
        return Err("Повреждённый файл".into());
    }
    let salt = &buf[0..16];
    let iv = &buf[16..28];
    let tag = &buf[28..44];
    let body = &buf[44..];
    let key = derive_key(password, salt);
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
        let key = derive_key("pw", &[7u8; 16]);
        let packed = aes_encrypt("hello", &key).expect("шифрование");
        assert_eq!(aes_decrypt(&packed, &key).expect("расшифровка"), "hello");
    }

    #[test]
    fn aes_decrypt_with_other_key_fails() {
        let k1 = derive_key("pw", &[1u8; 16]);
        let k2 = derive_key("pw", &[2u8; 16]);
        let packed = aes_encrypt("hello", &k1).expect("шифрование");
        assert!(aes_decrypt(&packed, &k2).is_err());
    }

    #[test]
    fn derive_key_is_deterministic_and_salt_sensitive() {
        // Та же пара (пароль, соль) — тот же ключ, иначе хранилище не откроется
        // после перезапуска. Разная соль — разный ключ, иначе соль бесполезна.
        assert_eq!(derive_key("pw", &[9u8; 16]), derive_key("pw", &[9u8; 16]));
        assert_ne!(derive_key("pw", &[9u8; 16]), derive_key("pw", &[8u8; 16]));
        assert_ne!(derive_key("pw1", &[9u8; 16]), derive_key("pw2", &[9u8; 16]));
    }

    #[test]
    fn corrupted_package_fails_gracefully() {
        // Битый файл хранилища должен давать Err, а не панику: паника в Tauri
        // роняет команду целиком и пользователь видит пустое окно без объяснения.
        let key = derive_key("pw", &[3u8; 16]);
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
