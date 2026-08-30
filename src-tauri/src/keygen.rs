//! Генерация SSH-ключей (порт keygen.ts) через crate ssh-key.

use serde_json::{json, Value};
use ssh_key::private::{Ed25519Keypair, RsaKeypair};
use ssh_key::{LineEnding, PrivateKey};

pub fn generate(params: &Value) -> Result<Value, String> {
    let ktype = params.get("type").and_then(|v| v.as_str()).unwrap_or("ed25519");
    let comment = params.get("comment").and_then(|v| v.as_str()).unwrap_or("");
    let passphrase = params
        .get("passphrase")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let mut rng = rand::rngs::OsRng;

    let mut key = if ktype == "rsa" {
        let bits = params.get("bits").and_then(|v| v.as_u64()).unwrap_or(4096) as usize;
        let kp = RsaKeypair::random(&mut rng, bits).map_err(|e| e.to_string())?;
        PrivateKey::from(kp)
    } else {
        let kp = Ed25519Keypair::random(&mut rng);
        PrivateKey::from(kp)
    };
    if !comment.is_empty() {
        key.set_comment(comment);
    }
    let final_key = match passphrase {
        Some(pass) => key.encrypt(&mut rng, pass).map_err(|e| e.to_string())?,
        None => key,
    };
    let private = final_key
        .to_openssh(LineEnding::LF)
        .map_err(|e| e.to_string())?
        .to_string();
    let public = final_key.public_key().to_openssh().map_err(|e| e.to_string())?;
    Ok(json!({ "privateKey": private, "publicKey": public }))
}

fn ensure_nl(s: &str) -> String {
    if s.ends_with('\n') {
        s.to_string()
    } else {
        format!("{s}\n")
    }
}

/// Закрыть приватный ключ от всех, кроме владельца.
///
/// На unix это `0600` — как того и требует OpenSSH. На Windows прав у файла нет, есть
/// список доступа, и он наследуется от каталога: ключ, сохранённый мимо профиля
/// пользователя (общая папка, диск с данными), окажется читаем шире, чем нужно.
/// Тот же OpenSSH под Windows потом откажется такой ключ использовать.
///
/// Правим через `icacls`: снимаем наследование и оставляем полный доступ только текущему
/// пользователю. Аргументы передаём отдельными, без оболочки, — путь может содержать
/// пробелы и любые символы.
fn restrict_private_key(path: &str) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let Ok(user) = std::env::var("USERNAME") else { return };
        let _ = std::process::Command::new("icacls")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("{user}:F"))
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
}

pub fn save_to(path: &str, key: &Value) -> Result<Value, String> {
    let private = key.get("privateKey").and_then(|v| v.as_str()).unwrap_or("");
    let public = key.get("publicKey").and_then(|v| v.as_str()).unwrap_or("");
    let pub_path = format!("{path}.pub");
    std::fs::write(path, ensure_nl(private)).map_err(|e| e.to_string())?;
    std::fs::write(&pub_path, ensure_nl(public)).map_err(|e| e.to_string())?;
    restrict_private_key(path);
    Ok(json!({ "saved": true, "privatePath": path, "publicPath": pub_path }))
}

/// Команда установки публичного ключа на сервер (аналог ssh-copy-id).
pub fn install_cmd(public_key: &str) -> String {
    let pubk = public_key.trim().replace('\'', "'\\''");
    format!(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys && grep -qxF '{pubk}' ~/.ssh/authorized_keys || echo '{pubk}' >> ~/.ssh/authorized_keys"
    )
}
