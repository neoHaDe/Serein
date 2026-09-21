//! Блокировки std::sync::Mutex без паники на отравленном мьютексе.
//!
//! Если поток умер, держа замок, Rust помечает мьютекс «отравленным». Паника здесь
//! уронила бы всё приложение из-за сбоя в одной фоновой задаче. Вместо этого
//! забираем данные из `PoisonError` и продолжаем - состояние могло частично
//! измениться, но это всё равно лучше, чем мгновенный выход.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

static POISONED: AtomicU64 = AtomicU64::new(0);

pub fn lock<'a, T>(m: &'a Mutex<T>) -> MutexGuard<'a, T> {
    m.lock().unwrap_or_else(|e| {
        // Отравленный замок - след паники в критической секции: правка состояния могла
        // остаться на половине. Продолжать всё равно лучше, чем валить приложение, но
        // молчать нельзя - иначе странное поведение потом не связать с той паникой.
        // Сюда нельзя звать журнал действий: он сам берёт замки этой же функцией.
        let n = POISONED.fetch_add(1, Ordering::Relaxed) + 1;
        eprintln!("замок отравлен ({n}): состояние осталось от прерванной паникой правки");
        e.into_inner()
    })
}

/// Сколько раз замки доставались отравленными. Видно в окне журнала действий.
pub fn poisoned_count() -> u64 {
    POISONED.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn recovers_after_poison() {
        let было = poisoned_count();
        let m = Arc::new(Mutex::new(42));
        let m2 = m.clone();
        let _ = thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("test poison");
        })
        .join();
        assert_eq!(*lock(&m), 42);
        assert!(poisoned_count() > было, "отравленный замок должен попадать в счётчик");
    }
}
