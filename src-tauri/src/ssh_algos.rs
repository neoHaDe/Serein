//! Согласование алгоритмов SSH: сжатие и режим совместимости со старым железом.
//!
//! Наборы russh по умолчанию рассчитаны на современный OpenSSH: в них нет
//! `diffie-hellman-group1-sha1`, CBC-шифров, `3des-cbc` и `ssh-rsa`. К коммутатору Cisco
//! или старому Dropbear с такими наборами не подключиться — а это ровно те устройства,
//! ради которых держат PuTTY. Поэтому профиль может включить legacy-режим: старые
//! алгоритмы добавляются **в конец** списков, так что с современным сервером
//! по-прежнему выбирается сильный набор.

use russh::cipher;
use russh::compression;
use russh::kex;
use russh::Preferred;
use russh_keys::key;
use serde_json::Value;
use std::borrow::Cow;

/// Устаревшие алгоритмы обмена ключами: без них не отвечает старое сетевое железо.
const LEGACY_KEX: &[kex::Name] = &[kex::DH_G14_SHA1, kex::DH_G1_SHA1];
/// CBC и 3DES — единственное, что понимают многие старые прошивки.
const LEGACY_CIPHER: &[cipher::Name] = &[
    cipher::AES_256_CBC,
    cipher::AES_192_CBC,
    cipher::AES_128_CBC,
    cipher::TRIPLE_DES_CBC,
];
/// RSA с подписью SHA-1 — до RFC 8332 других вариантов у RSA не было.
const LEGACY_HOST_KEY: &[key::Name] = &[key::SSH_RSA];

fn flag(server: &Value, name: &str) -> bool {
    server.get(name).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Собирает набор алгоритмов под конкретный сервер.
pub fn preferred_for(server: &Value) -> Preferred {
    let base = Preferred::DEFAULT;
    let legacy = flag(server, "sshLegacyAlgos");

    let kexes = if legacy {
        let mut v = base.kex.to_vec();
        v.extend_from_slice(LEGACY_KEX);
        Cow::Owned(v)
    } else {
        base.kex.clone()
    };

    let ciphers = if legacy {
        let mut v = base.cipher.to_vec();
        v.extend_from_slice(LEGACY_CIPHER);
        Cow::Owned(v)
    } else {
        base.cipher.clone()
    };

    let keys = if legacy {
        let mut v = base.key.to_vec();
        v.extend_from_slice(LEGACY_HOST_KEY);
        Cow::Owned(v)
    } else {
        base.key.clone()
    };

    // Сжатие: в наборе по умолчанию `none` стоит первым, поэтому фактически не включается.
    // Просьба сжимать = поставить zlib вперёд, оставив `none` запасным вариантом.
    let compression = if flag(server, "sshCompression") {
        Cow::Owned(vec![
            compression::ZLIB,
            compression::ZLIB_LEGACY,
            compression::NONE,
        ])
    } else {
        base.compression.clone()
    };

    Preferred {
        kex: kexes,
        key: keys,
        cipher: ciphers,
        mac: base.mac.clone(),
        compression,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names<T: AsRef<str> + Copy>(list: &[T]) -> Vec<String> {
        list.iter().map(|n| n.as_ref().to_string()).collect()
    }

    #[test]
    fn default_profile_keeps_modern_only() {
        let p = preferred_for(&json!({}));
        let kex = names(&p.kex);
        assert!(kex.iter().any(|k| k == "curve25519-sha256"));
        assert!(
            !kex.iter().any(|k| k == "diffie-hellman-group1-sha1"),
            "без просьбы о совместимости старые алгоритмы не предлагаем"
        );
        assert!(!names(&p.cipher).iter().any(|c| c == "3des-cbc"));
        assert!(!names(&p.key).iter().any(|k| k == "ssh-rsa"));
    }

    #[test]
    fn legacy_mode_adds_old_algorithms_at_the_end() {
        let p = preferred_for(&json!({ "sshLegacyAlgos": true }));
        let kex = names(&p.kex);
        let cipher = names(&p.cipher);
        let keys = names(&p.key);

        assert!(kex.iter().any(|k| k == "diffie-hellman-group1-sha1"));
        assert!(cipher.iter().any(|c| c == "3des-cbc"));
        assert!(keys.iter().any(|k| k == "ssh-rsa"));

        // Порядок = приоритет: с современным сервером договоримся о сильном наборе.
        let modern = kex.iter().position(|k| k == "curve25519-sha256").unwrap();
        let old = kex.iter().position(|k| k == "diffie-hellman-group1-sha1").unwrap();
        assert!(modern < old, "сильный обмен ключами должен идти первым");

        let strong = cipher.iter().position(|c| c == "chacha20-poly1305@openssh.com").unwrap();
        let weak = cipher.iter().position(|c| c == "3des-cbc").unwrap();
        assert!(strong < weak, "сильный шифр должен идти первым");
    }

    #[test]
    fn compression_moves_zlib_ahead_of_none() {
        let off = preferred_for(&json!({}));
        assert_eq!(names(&off.compression).first().map(String::as_str), Some("none"));

        let on = preferred_for(&json!({ "sshCompression": true }));
        let list = names(&on.compression);
        assert_eq!(list.first().map(String::as_str), Some("zlib"));
        assert!(
            list.iter().any(|c| c == "none"),
            "без запасного `none` сервер без сжатия отвалится"
        );
    }

    #[test]
    fn mac_list_is_untouched() {
        // MAC не трогаем: hmac-sha1 в наборе по умолчанию уже есть.
        let p = preferred_for(&json!({ "sshLegacyAlgos": true }));
        assert!(names(&p.mac).iter().any(|m| m == "hmac-sha1"));
    }
}
