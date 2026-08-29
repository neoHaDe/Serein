//! OS-layer for secrets: DPAPI on Windows, Secret Service (keyring) on Unix.

#[cfg(unix)]
const SERVICE: &str = "dev.serein.app";
const KR_PREFIX: &str = "kr:";

pub fn protect(plaintext: &str) -> Option<String> {
    #[cfg(windows)]
    {
        use base64::{engine::general_purpose::STANDARD, Engine};
        crate::dpapi::protect(plaintext.as_bytes())
            .ok()
            .map(|b| STANDARD.encode(b))
    }
    #[cfg(unix)]
    {
        protect_keyring(plaintext).or_else(|| {
            if plaintext.starts_with("mk:") {
                Some(plaintext.to_string())
            } else {
                None
            }
        })
    }
}

pub fn unprotect(stored: &str) -> Option<String> {
    if let Some(id) = stored.strip_prefix(KR_PREFIX) {
        #[cfg(unix)]
        {
            return unprotect_keyring(id);
        }
        #[cfg(not(unix))]
        {
            let _ = id;
            return None;
        }
    }
    if stored.starts_with("mk:") {
        return Some(stored.to_string());
    }
    #[cfg(windows)]
    {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let bytes = STANDARD.decode(stored).ok()?;
        String::from_utf8(crate::dpapi::unprotect(&bytes).ok()?).ok()
    }
    #[cfg(not(windows))]
    {
        let _ = stored;
        None
    }
}

/// Забыть секрет в OS-хранилище.
///
/// На Windows звать нечего: DPAPI ничего не хранит, в файле лежит сам шифртекст, и он
/// исчезает вместе с записью. На Linux полезная нагрузка живёт в связке ключей, а в файле
/// только ссылка `kr:{uuid}` — если запись не удалять, связка копит мусор: каждая правка
/// пароля заводит новую, а старая остаётся навсегда и её уже никто не найдёт.
pub fn forget(stored: &str) {
    #[cfg(unix)]
    {
        if let Some(id) = stored.strip_prefix(KR_PREFIX) {
            use keyring::Entry;
            if let Ok(entry) = Entry::new(SERVICE, &format!("secret:{id}")) {
                let _ = entry.delete_credential();
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = stored;
    }
}

#[cfg(unix)]
fn protect_keyring(plaintext: &str) -> Option<String> {
    use keyring::Entry;
    let id = uuid::Uuid::new_v4().to_string();
    let entry = Entry::new(SERVICE, &format!("secret:{id}")).ok()?;
    entry.set_password(plaintext).ok()?;
    Some(format!("{KR_PREFIX}{id}"))
}

#[cfg(unix)]
fn unprotect_keyring(id: &str) -> Option<String> {
    use keyring::Entry;
    let entry = Entry::new(SERVICE, &format!("secret:{id}")).ok()?;
    entry.get_password().ok()
}
