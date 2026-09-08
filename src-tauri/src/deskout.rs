//! Куда сейчас уходят кадры рабочего стола и какой сеанс жив у какого сервера.
//!
//! Появилось из-за открепления окна. Сеанс VNC или RDP живёт в этом процессе, а рисуют
//! его окна - и окно может смениться: панель открепляют, и картинка должна продолжиться
//! во втором окне, а не начаться заново с ввода пароля. Раньше канал выдачи кадров
//! запирался внутри задачи чтения намертво, и переезд был невозможен в принципе.
//!
//! Отсюда две вещи здесь. Первая - подменяемый приёмник кадров: задача чтения спрашивает
//! его на каждом кадре, а не держит копию. Вторая - список живых рабочих столов по
//! серверам: открывшаяся панель должна узнать, что подключаться заново не нужно.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::ipc::{Channel, InvokeResponseBody};

/// Приёмник кадров, который можно сменить не прерывая сеанса.
#[derive(Clone)]
pub struct Out(Arc<Mutex<Channel<InvokeResponseBody>>>);

impl Out {
    pub fn new(ch: Channel<InvokeResponseBody>) -> Self {
        Self(Arc::new(Mutex::new(ch)))
    }

    /// Переводит выдачу кадров в другое окно. Сеанс при этом не прерывается.
    pub fn set(&self, ch: Channel<InvokeResponseBody>) {
        *crate::sync::lock(&self.0) = ch;
    }

    /// Отправляет кадр текущему приёмнику. Ошибка значит «окно больше не слушает».
    pub fn send(&self, body: InvokeResponseBody) -> Result<(), ()> {
        crate::sync::lock(&self.0).send(body).map_err(|_| ())
    }
}

/// Каким способом открыт живой рабочий стол.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Vnc,
    Rdp,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Vnc => "vnc",
            Kind::Rdp => "rdp",
        }
    }
}

/// Живой рабочий стол: чем открыт, под каким номером и какого размера экран.
///
/// Размер нужен именно здесь: окно, которое подхватывает сеанс, начинает с пустого
/// холста и должно узнать, какой он величины, ещё до первого кадра от сервера.
#[derive(Clone)]
pub struct Active {
    pub kind: Kind,
    pub id: String,
    pub size: (u16, u16),
}

static ACTIVE: Mutex<Option<HashMap<String, Active>>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut HashMap<String, Active>) -> T) -> T {
    let mut g = crate::sync::lock(&ACTIVE);
    f(g.get_or_insert_with(HashMap::new))
}

/// Запоминает, что у этой SSH-сессии открыт рабочий стол.
///
/// Один на сессию: открывая второй, первый закрывают - так устроена и сама панель.
pub fn remember(ssh_id: &str, kind: Kind, desk_id: &str) {
    with(|m| {
        m.insert(
            ssh_id.to_owned(),
            Active { kind, id: desk_id.to_owned(), size: (0, 0) },
        )
    });
}

/// Обновляет запомненный размер экрана. Зовётся, когда сервер его называет.
pub fn note_size(desk_id: &str, w: u16, h: u16) {
    with(|m| {
        if let Some(a) = m.values_mut().find(|a| a.id == desk_id) {
            a.size = (w, h);
        }
    });
}

/// Размер экрана уже открытого стола - по его собственному номеру.
///
/// Нужен при переезде в другое окно: холст там создаётся до первого кадра.
pub fn active_size(desk_id: &str) -> Option<(u16, u16)> {
    with(|m| {
        m.values()
            .find(|a| a.id == desk_id)
            .map(|a| a.size)
            .filter(|(w, h)| *w > 0 && *h > 0)
    })
}

/// Забывает закрывшийся рабочий стол.
pub fn forget(desk_id: &str) {
    with(|m| m.retain(|_, a| a.id != desk_id));
}

/// Что открыто у этой SSH-сессии прямо сейчас.
pub fn active(ssh_id: &str) -> Option<Active> {
    with(|m| m.get(ssh_id).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn живой_стол_находится_по_сессии_и_забывается_по_себе() {
        remember("ssh-1", Kind::Rdp, "rdp-42");
        note_size("rdp-42", 1600, 900);
        let a = active("ssh-1").expect("стол должен найтись");
        assert_eq!(a.kind, Kind::Rdp);
        assert_eq!(a.id, "rdp-42");
        assert_eq!(a.size, (1600, 900), "размер нужен окну до первого кадра");

        // Забываем по номеру стола, а не по сессии: закрыться он может сам, и сессия
        // при этом остаётся живой.
        forget("rdp-42");
        assert!(active("ssh-1").is_none());
    }

    #[test]
    fn чужой_размер_не_приписывается() {
        remember("ssh-2", Kind::Vnc, "vnc-1");
        note_size("другой-стол", 800, 600);
        assert_eq!(active("ssh-2").unwrap().size, (0, 0));
        forget("vnc-1");
    }
}
