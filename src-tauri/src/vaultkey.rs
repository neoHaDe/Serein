//! Производный ключ мастер-пароля в памяти процесса. None — заблокировано/не задано.
//!
//! Ключ живёт до блокировки или выхода, поэтому его затирают явно: просто присвоить `None`
//! мало — байты остались бы в куче до случайной перезаписи и попали бы в дамп памяти.

use std::sync::Mutex;
use zeroize::Zeroize;

static MASTER_KEY: Mutex<Option<[u8; 32]>> = Mutex::new(None);

/// Копия ключа, которая затирает себя, когда её перестают использовать.
///
/// Возвращаем именно её, а не голый массив: иначе каждый вызывающий оставлял бы
/// свою копию ключа на стеке, и вся аккуратность с хранилищем теряла бы смысл.
pub struct MasterKey([u8; 32]);

impl std::ops::Deref for MasterKey {
    type Target = [u8; 32];
    fn deref(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub fn get() -> Option<MasterKey> {
    crate::sync::lock(&MASTER_KEY).map(MasterKey)
}

pub fn set(key: Option<[u8; 32]>) {
    let mut guard = crate::sync::lock(&MASTER_KEY);
    // Прежний ключ затираем на месте, до замены: иначе он останется в памяти
    // и после блокировки хранилища.
    if let Some(old) = guard.as_mut() {
        old.zeroize();
    }
    *guard = key;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locking_clears_the_stored_key() {
        set(Some([7u8; 32]));
        assert_eq!(*get().expect("ключ должен быть задан"), [7u8; 32]);
        set(None);
        assert!(get().is_none(), "после блокировки ключа быть не должно");
    }
}
