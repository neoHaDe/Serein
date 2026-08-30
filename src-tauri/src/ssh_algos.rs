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
use russh::mac;
use russh::kex;
use russh::Preferred;
use russh::keys::Algorithm;
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
/// HMAC-SHA1 — единственный MAC, который понимает часть старых прошивок.
///
/// В russh 0.45 он входил в набор по умолчанию, и legacy-режим мог его не трогать.
/// В 0.63 его из умолчания убрали — правильно для современных серверов, но для старого
/// железа это значит «не подключиться вовсе». Поэтому теперь добавляем явно, как kex и
/// шифры: в конец списка, чтобы с нормальным сервером выбрался сильный вариант.
const LEGACY_MAC: &[mac::Name] = &[mac::HMAC_SHA1_ETM, mac::HMAC_SHA1];
/// RSA с подписью SHA-1 — до RFC 8332 других вариантов у RSA не было.
///
/// В russh 0.63 `ssh-rsa` уже входит в набор по умолчанию (последним), так что добавление
/// обычно ничего не меняет. Список оставлен явным намеренно: если умолчание однажды
/// поменяется, legacy-режим не должен молча перестать работать со старым железом.
const LEGACY_HOST_KEY: &[Algorithm] = &[Algorithm::Rsa { hash: None }];

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

    // ⚠ В russh 0.63 `ssh-rsa` (RSA с подписью SHA-1) входит в набор по умолчанию — в 0.45
    // его там не было. Молча принять это изменение значило бы ослабить проверку ключа
    // хоста на всех профилях сразу: мы бы снова соглашались на SHA-1 там, где раньше
    // требовали rsa-sha2. Поэтому в обычном режиме убираем его явно, а в legacy —
    // наоборот, гарантируем присутствие: ради старого железа этот режим и существует.
    let keys = if legacy {
        let mut v = base.key.to_vec();
        // Без проверки на дубль набор по умолчанию и legacy-список пересекаются,
        // и сервер получил бы `ssh-rsa` дважды.
        let extra: Vec<Algorithm> =
            LEGACY_HOST_KEY.iter().filter(|a| !v.contains(a)).cloned().collect();
        v.extend(extra);
        Cow::Owned(v)
    } else {
        Cow::Owned(
            base.key
                .iter()
                .filter(|a| !LEGACY_HOST_KEY.contains(a))
                .cloned()
                .collect::<Vec<_>>(),
        )
    };

    let macs = if legacy {
        let mut v = base.mac.to_vec();
        v.extend(LEGACY_MAC.iter().filter(|m| !v.contains(m)).copied().collect::<Vec<_>>());
        Cow::Owned(v)
    } else {
        base.mac.clone()
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
        // Сертификаты хостов не запрашиваем: доверять им можно, только зная набор
        // доверенных удостоверяющих ключей, а его у нас нет. Пустой список — как в
        // умолчании russh.
        host_key_certificates: base.host_key_certificates.clone(),
        cipher: ciphers,
        mac: macs,
        compression,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names<T: AsRef<str>>(list: &[T]) -> Vec<String> {
        list.iter().map(|n| n.as_ref().to_string()).collect()
    }

    /// `Algorithm` — не строка: у него есть только Display, и он не `Copy`.
    fn algos(list: &[Algorithm]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
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
        assert!(!algos(&p.key).iter().any(|k| k == "ssh-rsa"));
    }

    #[test]
    fn legacy_mode_adds_old_algorithms_at_the_end() {
        let p = preferred_for(&json!({ "sshLegacyAlgos": true }));
        let kex = names(&p.kex);
        let cipher = names(&p.cipher);
        let keys = algos(&p.key);

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
    fn legacy_mode_adds_hmac_sha1_but_default_does_not() {
        // russh 0.63 убрал hmac-sha1 из набора по умолчанию. Для старого железа это значит
        // «не подключиться вовсе», поэтому legacy-режим обязан его вернуть — но только он.
        let legacy = preferred_for(&json!({ "sshLegacyAlgos": true }));
        let macs = names(&legacy.mac);
        assert!(macs.iter().any(|m| m == "hmac-sha1"));
        assert!(
            macs.iter().position(|m| m == "hmac-sha2-512-etm@openssh.com").unwrap()
                < macs.iter().position(|m| m == "hmac-sha1").unwrap(),
            "старый MAC должен стоять в конце, а не вытеснять сильный"
        );

        let modern = preferred_for(&json!({}));
        assert!(!names(&modern.mac).iter().any(|m| m == "hmac-sha1"));
    }

    #[test]
    fn default_profile_does_not_offer_sha1_rsa_host_keys() {
        // russh 0.63 вернул `ssh-rsa` в набор по умолчанию. Без просьбы о совместимости
        // мы его не предлагаем: иначе проверка ключа хоста молча съезжает на SHA-1.
        assert!(!algos(&preferred_for(&json!({})).key).iter().any(|k| k == "ssh-rsa"));
        assert!(algos(&preferred_for(&json!({ "sshLegacyAlgos": true })).key)
            .iter()
            .any(|k| k == "ssh-rsa"));
    }
}
