/**
 * Перевод клавиатурных событий браузера в X11 keysym — то, чего ждёт протокол RFB.
 *
 * Вынесено отдельно и покрыто тестами, потому что ошибка здесь не выглядит как ошибка:
 * приложение работает, экран рисуется, а на сервер уезжает не та буква — и заметить это
 * можно только руками, на конкретной раскладке.
 */

/** Клавиши, у которых нет символа: их keysym задан таблицей X11 (`keysymdef.h`). */
const NAMED: Record<string, number> = {
  Backspace: 0xff08,
  Tab: 0xff09,
  Enter: 0xff0d,
  Escape: 0xff1b,
  Home: 0xff50,
  ArrowLeft: 0xff51,
  ArrowUp: 0xff52,
  ArrowRight: 0xff53,
  ArrowDown: 0xff54,
  PageUp: 0xff55,
  PageDown: 0xff56,
  End: 0xff57,
  Insert: 0xff63,
  NumLock: 0xff7f,
  Delete: 0xffff,
  ShiftLeft: 0xffe1,
  ShiftRight: 0xffe2,
  ControlLeft: 0xffe3,
  ControlRight: 0xffe4,
  CapsLock: 0xffe5,
  AltLeft: 0xffe9,
  AltRight: 0xffea,
  MetaLeft: 0xffeb,
  MetaRight: 0xffec,
  ContextMenu: 0xff67,
  PrintScreen: 0xff61,
  Pause: 0xff13,
  ScrollLock: 0xff14
}

/** Начало диапазона функциональных клавиш: F1 = 0xffbe, дальше по порядку. */
const F1 = 0xffbe
/** Юникод за пределами Latin-1 кодируется как 0x01000000 + кодовая точка. */
const UNICODE_BASE = 0x01000000

/**
 * Кириллица в X11 живёт не в юникод-диапазоне, а в своём — унаследованном от КОИ-8
 * (`keysym = 0x600 + байт КОИ-8`). Юникод-форму `0x01000000 + код` понимают не все серверы:
 * старые её просто игнорируют, и русский текст не набирается вовсе. Поэтому для кириллицы
 * отдаём исторические значения, а юникод оставляем всему остальному.
 */
const CYRILLIC: Record<string, number> = {
  'а': 0x6c1,
  'б': 0x6c2,
  'в': 0x6d7,
  'г': 0x6c7,
  'д': 0x6c4,
  'е': 0x6c5,
  'ё': 0x6a3,
  'ж': 0x6d6,
  'з': 0x6da,
  'и': 0x6c9,
  'й': 0x6ca,
  'к': 0x6cb,
  'л': 0x6cc,
  'м': 0x6cd,
  'н': 0x6ce,
  'о': 0x6cf,
  'п': 0x6d0,
  'р': 0x6d2,
  'с': 0x6d3,
  'т': 0x6d4,
  'у': 0x6d5,
  'ф': 0x6c6,
  'х': 0x6c8,
  'ц': 0x6c3,
  'ч': 0x6de,
  'ш': 0x6db,
  'щ': 0x6dd,
  'ъ': 0x6df,
  'ы': 0x6d9,
  'ь': 0x6d8,
  'э': 0x6dc,
  'ю': 0x6c0,
  'я': 0x6d1,
  'А': 0x6e1,
  'Б': 0x6e2,
  'В': 0x6f7,
  'Г': 0x6e7,
  'Д': 0x6e4,
  'Е': 0x6e5,
  'Ё': 0x6b3,
  'Ж': 0x6f6,
  'З': 0x6fa,
  'И': 0x6e9,
  'Й': 0x6ea,
  'К': 0x6eb,
  'Л': 0x6ec,
  'М': 0x6ed,
  'Н': 0x6ee,
  'О': 0x6ef,
  'П': 0x6f0,
  'Р': 0x6f2,
  'С': 0x6f3,
  'Т': 0x6f4,
  'У': 0x6f5,
  'Ф': 0x6e6,
  'Х': 0x6e8,
  'Ц': 0x6e3,
  'Ч': 0x6fe,
  'Ш': 0x6fb,
  'Щ': 0x6fd,
  'Ъ': 0x6ff,
  'Ы': 0x6f9,
  'Ь': 0x6f8,
  'Э': 0x6fc,
  'Ю': 0x6e0,
  'Я': 0x6f1
}

/**
 * Keysym для события клавиатуры. `null` — клавиша, которую посылать не нужно.
 *
 * Ориентируемся на `key`, а не на `code`: `code` — это физическая клавиша, и на кириллице
 * он останется `KeyF`, хотя пользователь набрал «а». Раскладку раскладывает сама система,
 * и результат её работы лежит именно в `key`.
 */
export function keysymFor(e: Pick<KeyboardEvent, 'key' | 'code'>): number | null {
  // Функциональные клавиши идут подряд, отдельной строкой в таблице их держать незачем.
  const fn = /^F([1-9]|1[0-9]|2[0-4])$/.exec(e.key)
  if (fn) return F1 + Number(fn[1]) - 1

  // У модификаторов `key` одинаковый слева и справа («Shift»), различает их только `code`.
  if (e.code in NAMED) return NAMED[e.code]
  if (e.key in NAMED) return NAMED[e.key]

  if (e.key === ' ') return 0x20
  if (e.key.length !== 1) return null

  const cyr = CYRILLIC[e.key]
  if (cyr !== undefined) return cyr

  const cp = e.key.codePointAt(0)!
  // ASCII и Latin-1 совпадают с keysym один в один — исторически они из него и выросли.
  if (cp <= 0xff) return cp
  return UNICODE_BASE + cp
}

/** Кнопки мыши в маске RFB: 1 левая, 2 средняя, 4 правая. */
export function buttonMask(browserButtons: number): number {
  let mask = 0
  if (browserButtons & 1) mask |= 1
  if (browserButtons & 4) mask |= 2
  if (browserButtons & 2) mask |= 4
  return mask
}

/** Колесо мыши в RFB — это «нажатие» кнопок 4 и 5, отдельного события прокрутки нет. */
export function wheelMask(deltaY: number, deltaX = 0): number {
  let mask = 0
  if (deltaY < 0) mask |= 8
  if (deltaY > 0) mask |= 16
  if (deltaX < 0) mask |= 32
  if (deltaX > 0) mask |= 64
  return mask
}
