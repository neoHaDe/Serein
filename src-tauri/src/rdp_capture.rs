//! Перехват системных сочетаний Windows для удалённого стола.
//!
//! Обработчик клавиш в окне видит Ctrl+W и Tab, но не Win, Alt+Tab и Ctrl+Esc: их Windows
//! забирает раньше, чем событие дойдёт до окна. Перехватить их можно только низкоуровневым
//! хуком клавиатуры, и только пока окно Serein впереди.
//!
//! Хук ставится лишь на время, когда в фокусе холст живого RDP и включена настройка, и
//! снимается при потере фокуса, закрытии сеанса или выключении настройки. Пока он стоит,
//! все клавиши с известным скан-кодом идут на удалённый стол отсюда и в одном порядке: если
//! бы модификаторы шли через хук, а буквы через окно, Ctrl мог бы прийти на сервер после C.
//!
//! Обработчик хука только решает и кладёт событие в ограниченную очередь: хук, который
//! думает дольше предела, Windows снимает молча. Отправка - в отдельном потоке.
#![cfg_attr(not(windows), allow(dead_code))]

/// Что делать с клавишей.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Decision {
    /// Не трогать: получат Windows и окно.
    Pass,
    /// Проглотить и отправить на удалённый стол с этим скан-кодом.
    Send(u16),
    /// Проглотить, ничего не отправляя.
    Drop,
    /// Ctrl+Alt+Pause или Ctrl+Alt+Home: вернуть клавиатуру Windows.
    Release,
}

/// Что уходит из обработчика хука в поток отправки.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Out {
    Key(u16, bool),
    Release,
}

const LLKHF_EXTENDED: u32 = 0x01;
const LLKHF_INJECTED: u32 = 0x10;
const LLKHF_UP: u32 = 0x80;
const VK_CANCEL: u32 = 0x03;
const VK_PAUSE: u32 = 0x13;
const VK_HOME: u32 = 0x24;
const VK_SNAPSHOT: u32 = 0x2C;
const VK_NUMLOCK: u32 = 0x90;
const VK_RSHIFT: u32 = 0xA1;

/// Решение по событию хука. Скан-коды - в той же нумерации, что у `rdpKeys.ts`.
pub fn decide(vk: u32, scan: u32, flags: u32, ctrl_alt: bool) -> Decision {
    // Синтетический ввод других программ - не пользовательская клавиатура.
    if flags & LLKHF_INJECTED != 0 {
        return Decision::Pass;
    }
    match vk {
        // Выход обязан быть: без него из полного экрана с перехваченным Esc не уйти.
        VK_PAUSE | VK_CANCEL | VK_HOME if ctrl_alt => return Decision::Release,
        VK_PAUSE | VK_CANCEL => return Decision::Pass,
        // Хук помечает NumLock расширенной, а правый Shift - как придётся; у RDP обе обычные.
        VK_NUMLOCK => return Decision::Send(0x45),
        VK_RSHIFT => return Decision::Send(0x36),
        VK_SNAPSHOT => return Decision::Send(0xE037),
        _ => {}
    }
    // AltGr Windows сопровождает фальшивым левым Ctrl со скан-кодом 0x21D. Сервер сам сделает
    // то же из правого Alt, а лишний Ctrl превратил бы AltGr в Ctrl+Alt.
    if scan & 0x200 != 0 {
        return Decision::Drop;
    }
    if scan == 0 || scan > 0x7F {
        return Decision::Pass;
    }
    let code = scan as u16;
    Decision::Send(if flags & LLKHF_EXTENDED != 0 { 0xE000 | code } else { code })
}

/// Модификатор: его авто-повтор не отправляем, см. `isModifier` в `rdpKeys.ts`.
fn is_modifier(code: u16) -> bool {
    matches!(code, 0x1d | 0xE01D | 0x2a | 0x36 | 0x38 | 0xE038 | 0xE05B | 0xE05C | 0x3a | 0x45 | 0x46)
}

/// Что нажато на удалённом столе через хук.
#[derive(Default, Debug)]
pub struct Keys {
    held: Vec<u16>,
}

impl Keys {
    fn ctrl_alt(&self) -> bool {
        let has = |a: u16, b: u16| self.held.iter().any(|c| *c == a || *c == b);
        has(0x1d, 0xE01D) && has(0x38, 0xE038)
    }

    /// Проглотить ли событие и что отправить.
    pub fn on_event(&mut self, vk: u32, scan: u32, flags: u32) -> (bool, Option<Out>) {
        let up = flags & LLKHF_UP != 0;
        match decide(vk, scan, flags, self.ctrl_alt()) {
            Decision::Pass => (false, None),
            Decision::Drop => (true, None),
            Decision::Release => (true, (!up).then_some(Out::Release)),
            Decision::Send(code) if up => match self.held.iter().position(|c| *c == code) {
                Some(i) => {
                    self.held.swap_remove(i);
                    (true, Some(Out::Key(code, false)))
                }
                // Нажата до перехвата: нажатие отправило окно, пусть оно же отправит и отпускание.
                None => (false, None),
            },
            Decision::Send(code) if self.held.contains(&code) => {
                (true, (!is_modifier(code)).then_some(Out::Key(code, true)))
            }
            Decision::Send(code) => {
                self.held.push(code);
                (true, Some(Out::Key(code, true)))
            }
        }
    }

    /// Нажатие не ушло - забыть о нём, чтобы клавиша прошла Windows целиком.
    pub fn forget(&mut self, code: u16) {
        self.held.retain(|c| *c != code);
    }

    pub fn held(&self) -> Vec<u16> {
        self.held.clone()
    }
}

#[cfg(windows)]
mod imp {
    use super::{Keys, Out};
    use std::sync::mpsc::{sync_channel, SyncSender};
    use std::sync::{Mutex, MutexGuard};
    use tauri::{AppHandle, Emitter};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetForegroundWindow, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
        UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WM_QUIT, WM_USER,
    };

    /// Очередь к потоку отправки. Отправка - строка в канал сеанса, так что заполниться она
    /// может разве что при зависшем процессе; тогда нажатие отдаём Windows, а не теряем.
    const QUEUE: usize = 256;

    struct Active {
        id: String,
        hwnd: isize,
        thread: u32,
        tx: SyncSender<Out>,
        keys: Keys,
    }

    static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

    fn lock() -> MutexGuard<'static, Option<Active>> {
        ACTIVE.lock().unwrap_or_else(|e| e.into_inner())
    }

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let ev = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            if swallow(ev) {
                return LRESULT(1);
            }
        }
        CallNextHookEx(HHOOK::default(), code, wparam, lparam)
    }

    fn swallow(ev: &KBDLLHOOKSTRUCT) -> bool {
        // Замок держат только включение и выключение, и то мгновение: ждать его в хуке нельзя.
        let Ok(mut guard) = ACTIVE.try_lock() else { return false };
        let Some(a) = guard.as_mut() else { return false };
        // Впереди не окно Serein - клавиатура не наша.
        if unsafe { GetForegroundWindow() }.0 as isize != a.hwnd {
            return false;
        }
        let (swallow, out) = a.keys.on_event(ev.vkCode, ev.scanCode, ev.flags.0);
        match out {
            None => swallow,
            Some(Out::Key(code, true)) => {
                if a.tx.try_send(Out::Key(code, true)).is_ok() {
                    true
                } else {
                    a.keys.forget(code);
                    false
                }
            }
            // Отпускание терять нельзя - на сервере залипнет клавиша. Поток отправки замка
            // не берёт, поэтому ожидание здесь короткое и без взаимной блокировки.
            Some(other) => {
                let _ = a.tx.send(other);
                swallow
            }
        }
    }

    pub fn start(app: AppHandle, id: String, hwnd: isize) -> Result<(), String> {
        if lock().as_ref().is_some_and(|a| a.id == id && a.hwnd == hwnd) {
            return Ok(());
        }
        stop(None);
        let (tx, rx) = sync_channel::<Out>(QUEUE);
        let send_id = id.clone();
        std::thread::Builder::new()
            .name("rdp-keys".into())
            .spawn(move || {
                for out in rx {
                    match out {
                        Out::Key(code, down) => crate::rdp::key(&send_id, code, down),
                        Out::Release => {
                            let _ = app.emit("rdp-capture-release", &send_id);
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32, String>>();
        std::thread::Builder::new()
            .name("rdp-hook".into())
            .spawn(move || run_hook(ready_tx))
            .map_err(|e| e.to_string())?;
        let thread = ready_rx.recv().map_err(|_| "поток перехвата не запустился".to_owned())??;
        *lock() = Some(Active { id, hwnd, thread, tx, keys: Keys::default() });
        Ok(())
    }

    /// Хук живёт в своём потоке: Windows зовёт обработчик в потоке, который его поставил, и
    /// только пока тот крутит очередь сообщений.
    fn run_hook(ready: std::sync::mpsc::Sender<Result<u32, String>>) {
        unsafe {
            let mut msg = MSG::default();
            // Очередь сообщений потока появляется при первом обращении к ней; без этого
            // просьба уйти до потока не дошла бы.
            let _ = PeekMessageW(&mut msg, HWND::default(), WM_USER, WM_USER, PM_NOREMOVE);
            let module = match GetModuleHandleW(PCWSTR::null()) {
                Ok(m) => m,
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                    return;
                }
            };
            let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), HINSTANCE::from(module), 0) {
                Ok(h) => h,
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                    return;
                }
            };
            let _ = ready.send(Ok(GetCurrentThreadId()));
            while GetMessageW(&mut msg, HWND::default(), 0, 0).0 > 0 {}
            let _ = UnhookWindowsHookEx(hook);
        }
    }

    /// Снимает перехват - любой или только этого сеанса.
    pub fn stop(only: Option<&str>) {
        let taken = {
            let mut g = lock();
            if only.is_some_and(|id| g.as_ref().is_some_and(|a| a.id != id)) {
                return;
            }
            g.take()
        };
        if let Some(a) = taken {
            unsafe {
                let _ = PostThreadMessageW(a.thread, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            // Нажатое на удалённом столе отпускаем: иначе там залипнет Ctrl или Win.
            for code in a.keys.held() {
                let _ = a.tx.send(Out::Key(code, false));
            }
        }
    }
}

#[cfg(windows)]
pub use imp::{start, stop};

#[cfg(not(windows))]
pub fn start(_app: tauri::AppHandle, _id: String, _hwnd: isize) -> Result<(), String> {
    Err("перехват системных сочетаний есть только на Windows".into())
}

#[cfg(not(windows))]
pub fn stop(_only: Option<&str>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn клавиши_хука_переводятся_в_скан_коды_rdp() {
        assert_eq!(decide(0x41, 0x1e, 0, false), Decision::Send(0x1e), "A");
        assert_eq!(decide(0x5b, 0x5b, LLKHF_EXTENDED, false), Decision::Send(0xE05B), "Win");
        assert_eq!(decide(0x09, 0x0f, 0, false), Decision::Send(0x0f), "Tab");
        assert_eq!(decide(0xA5, 0x38, LLKHF_EXTENDED, false), Decision::Send(0xE038), "правый Alt");
        assert_eq!(decide(0x90, 0x45, LLKHF_EXTENDED, false), Decision::Send(0x45), "NumLock");
        assert_eq!(decide(0xA1, 0x36, LLKHF_EXTENDED, false), Decision::Send(0x36), "правый Shift");
        assert_eq!(decide(0xA2, 0x21D, 0, false), Decision::Drop, "фальшивый Ctrl от AltGr");
        assert_eq!(decide(0x41, 0x1e, LLKHF_INJECTED, false), Decision::Pass, "синтетический ввод");
        assert_eq!(decide(0x13, 0x45, 0, false), Decision::Pass, "Pause без Ctrl+Alt");
        assert_eq!(decide(0x24, 0x47, LLKHF_EXTENDED, false), Decision::Send(0xE047), "Home");
        assert_eq!(decide(0x24, 0x47, LLKHF_EXTENDED, true), Decision::Release);
    }

    #[test]
    fn alt_tab_уходит_на_сервер_а_повтор_модификатора_нет() {
        let mut k = Keys::default();
        assert_eq!(k.on_event(0xA4, 0x38, 0), (true, Some(Out::Key(0x38, true))));
        assert_eq!(k.on_event(0xA4, 0x38, 0), (true, None), "повтор Alt не отправляется");
        assert_eq!(k.on_event(0x09, 0x0f, 0), (true, Some(Out::Key(0x0f, true))));
        assert_eq!(k.on_event(0x09, 0x0f, 0), (true, Some(Out::Key(0x0f, true))), "повтор Tab - да");
        assert_eq!(k.on_event(0x09, 0x0f, LLKHF_UP), (true, Some(Out::Key(0x0f, false))));
        assert_eq!(k.held(), vec![0x38], "Alt ещё нажат - его отпустит снятие перехвата");
        assert_eq!(k.on_event(0x41, 0x1e, LLKHF_UP), (false, None), "нажатую до перехвата отпускает окно");
    }

    #[test]
    fn ctrl_alt_pause_возвращает_клавиатуру_windows() {
        let mut k = Keys::default();
        k.on_event(0xA2, 0x1d, 0);
        k.on_event(0xA4, 0x38, 0);
        assert_eq!(k.on_event(0x03, 0x46, LLKHF_EXTENDED), (true, Some(Out::Release)));
        assert_eq!(k.on_event(0x03, 0x46, LLKHF_EXTENDED | LLKHF_UP), (true, None));
        let mut held = k.held();
        held.sort_unstable();
        assert_eq!(held, vec![0x1d, 0x38], "Ctrl и Alt отпустятся при снятии перехвата");
    }
}
