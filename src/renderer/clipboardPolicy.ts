/**
 * Можно ли пустить в буфер обмена то, что прислал сервер.
 *
 * Последовательность OSC 52 - единственное место, где удалённая сторона меняет состояние
 * нашей машины без нашего действия. Поэтому решение вынесено сюда и покрыто тестами: цена
 * ошибки здесь не косметическая, а «вставил не то, что копировал».
 */

/** Шестьдесят четыре килобайта - это всё ещё «скопировали текст». */
export const CLIPBOARD_FROM_SERVER_MAX = 64 * 1024

/** Не чаще раза в полсекунды: поток последовательностей иначе просто занимает буфер. */
export const CLIPBOARD_FROM_SERVER_EVERY_MS = 500

export interface ClipboardAsk {
  /** Работает ли человек сейчас в этом терминале. */
  focused: boolean
  /** Длина присланного текста. */
  length: number
  /** Сколько прошло с прошлой такой записи, в миллисекундах. */
  sinceLastMs: number
}

/**
 * Фоновая вкладка буфер не меняет вообще.
 *
 * Иначе открытая рядом сессия - своя или чужая, если на сервере кто-то ещё, - подменяет
 * то, что вы собирались вставить, и заметить это можно только по последствиям.
 */
export function allowClipboardFromServer({ focused, length, sinceLastMs }: ClipboardAsk): boolean {
  if (!focused) return false
  if (length <= 0 || length > CLIPBOARD_FROM_SERVER_MAX) return false
  return sinceLastMs >= CLIPBOARD_FROM_SERVER_EVERY_MS
}
