//! Производный ключ мастер-пароля в памяти процесса. None — заблокировано/не задано.

use std::sync::Mutex;

static MASTER_KEY: Mutex<Option<[u8; 32]>> = Mutex::new(None);

pub fn get() -> Option<[u8; 32]> {
    *MASTER_KEY.lock().unwrap()
}

pub fn set(key: Option<[u8; 32]>) {
    *MASTER_KEY.lock().unwrap() = key;
}
