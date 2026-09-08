/**
 * Перевод клавиш браузера в скан-коды RDP.
 *
 * RDP говорит скан-кодами набора 1 - теми же, что PC/AT посылает по проводу. Это не то же
 * самое, что keysym у VNC (`vncKeys.ts`): там передаётся «какой символ», здесь - «какая
 * физическая клавиша». Поэтому таблица строится по `event.code`, а не по `event.key`:
 * `code` описывает место на клавиатуре и не зависит от раскладки, а раскладку сервер
 * применит сам, своей.
 *
 * Расширенные клавиши помечаются префиксом `0xE0`. Мы кладём его в старший байт, а
 * помощник разбирает: `Scancode::from_u16` в IronRDP считает расширенной ту, у которой
 * `код & 0xE000 == 0xE000`.
 */

/** Обычные клавиши: код без префикса. */
const PLAIN: Record<string, number> = {
  Escape: 0x01,
  Digit1: 0x02, Digit2: 0x03, Digit3: 0x04, Digit4: 0x05, Digit5: 0x06,
  Digit6: 0x07, Digit7: 0x08, Digit8: 0x09, Digit9: 0x0a, Digit0: 0x0b,
  Minus: 0x0c, Equal: 0x0d, Backspace: 0x0e,
  Tab: 0x0f,
  KeyQ: 0x10, KeyW: 0x11, KeyE: 0x12, KeyR: 0x13, KeyT: 0x14, KeyY: 0x15,
  KeyU: 0x16, KeyI: 0x17, KeyO: 0x18, KeyP: 0x19,
  BracketLeft: 0x1a, BracketRight: 0x1b, Enter: 0x1c,
  ControlLeft: 0x1d,
  KeyA: 0x1e, KeyS: 0x1f, KeyD: 0x20, KeyF: 0x21, KeyG: 0x22, KeyH: 0x23,
  KeyJ: 0x24, KeyK: 0x25, KeyL: 0x26,
  Semicolon: 0x27, Quote: 0x28, Backquote: 0x29,
  ShiftLeft: 0x2a, Backslash: 0x2b,
  KeyZ: 0x2c, KeyX: 0x2d, KeyC: 0x2e, KeyV: 0x2f, KeyB: 0x30, KeyN: 0x31, KeyM: 0x32,
  Comma: 0x33, Period: 0x34, Slash: 0x35, ShiftRight: 0x36,
  NumpadMultiply: 0x37, AltLeft: 0x38, Space: 0x39, CapsLock: 0x3a,
  F1: 0x3b, F2: 0x3c, F3: 0x3d, F4: 0x3e, F5: 0x3f,
  F6: 0x40, F7: 0x41, F8: 0x42, F9: 0x43, F10: 0x44,
  NumLock: 0x45, ScrollLock: 0x46,
  Numpad7: 0x47, Numpad8: 0x48, Numpad9: 0x49, NumpadSubtract: 0x4a,
  Numpad4: 0x4b, Numpad5: 0x4c, Numpad6: 0x4d, NumpadAdd: 0x4e,
  Numpad1: 0x4f, Numpad2: 0x50, Numpad3: 0x51, Numpad0: 0x52, NumpadDecimal: 0x53,
  IntlBackslash: 0x56, F11: 0x57, F12: 0x58
}

/** Расширенные: те же коды, но с префиксом `0xE0`. */
const EXTENDED: Record<string, number> = {
  NumpadEnter: 0x1c,
  ControlRight: 0x1d,
  NumpadDivide: 0x35,
  // На большинстве раскладок это правый Alt, но на европейских - AltGr, и сервер
  // различает их сам по префиксу. Наше дело - не перепутать со левым.
  AltRight: 0x38,
  Home: 0x47, ArrowUp: 0x48, PageUp: 0x49,
  ArrowLeft: 0x4b, ArrowRight: 0x4d,
  End: 0x4f, ArrowDown: 0x50, PageDown: 0x51,
  Insert: 0x52, Delete: 0x53,
  MetaLeft: 0x5b, MetaRight: 0x5c, ContextMenu: 0x5d
}

/**
 * Скан-код для клавиши или `null`, если такой у нас нет.
 *
 * `null` - это не ошибка, а честный ответ: клавиш на свете больше, чем в таблице, и
 * отправлять вместо неизвестной что-то похожее хуже, чем не отправлять ничего.
 */
export function scancodeFor(code: string): number | null {
  const plain = PLAIN[code]
  if (plain !== undefined) return plain
  const ext = EXTENDED[code]
  if (ext !== undefined) return 0xe000 | ext
  return null
}

/**
 * Маска кнопок мыши для RDP из маски браузера.
 *
 * Совпадает один в один - и левая, и правая, и средняя лежат на тех же битах. Функция
 * существует ради явности: молчаливое совпадение чужих числовых соглашений однажды
 * разъедется, и лучше, чтобы это место было названо.
 */
/**
 * Клавиша-модификатор, которую имеет смысл только держать.
 *
 * Нужна, чтобы не отправлять авто-повтор: у повторного нажатия Alt нет смысла, а вреда
 * много. Библиотека ввода RDP на каждый повтор добавляет отпускание и новое нажатие, и
 * посреди сочетания Alt+Shift это читается сервером как второе переключение раскладки -
 * она меняется и сразу возвращается обратно.
 */
export function isModifier(code: string): boolean {
  return (
    code === 'ShiftLeft' ||
    code === 'ShiftRight' ||
    code === 'ControlLeft' ||
    code === 'ControlRight' ||
    code === 'AltLeft' ||
    code === 'AltRight' ||
    code === 'MetaLeft' ||
    code === 'MetaRight' ||
    code === 'CapsLock' ||
    code === 'NumLock' ||
    code === 'ScrollLock'
  )
}

export function buttonMask(buttons: number): number {
  return buttons & 0b111
}
