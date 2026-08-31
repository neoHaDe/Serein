//! Блокировки std::sync::Mutex без паники на отравленном мьютексе.
//!
//! Если поток умер, держа замок, Rust помечает мьютекс «отравленным». Паника здесь
//! уронила бы всё приложение из-за сбоя в одной фоновой задаче. Вместо этого
//! забираем данные из `PoisonError` и продолжаем — состояние могло частично
//! измениться, но это всё равно лучше, чем мгновенный выход.

use std::sync::{Mutex, MutexGuard};

pub fn lock<'a, T>(m: &'a Mutex<T>) -> MutexGuard<'a, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn recovers_after_poison() {
        let m = Arc::new(Mutex::new(42));
        let m2 = m.clone();
        let _ = thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("test poison");
        })
        .join();
        assert_eq!(*lock(&m), 42);
    }
}
