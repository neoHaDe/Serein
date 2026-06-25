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
