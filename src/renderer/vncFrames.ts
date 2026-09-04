/**
 * Разбор бинарных пакетов рабочего стола, приходящих из Rust.
 *
 * Пиксели намеренно не ходят через JSON: кадр 1920×1080 — это восемь мегабайт, и в виде
 * массива чисел он раздувается на порядок. Поэтому здесь свой формат поверх ArrayBuffer,
 * а разбор вынесен из компонента, чтобы его можно было проверить тестами: рассинхрон
 * между этой таблицей и `src-tauri/src/vnc.rs` рисует мусор на экране, и отлаживать это
 * глазами — худший из возможных способов.
 *
 * Пакет: `[u8 тип][u16 x][u16 y][u16 w][u16 h][тело]`, порядок байтов сетевой.
 */

export const VNC_KIND = {
  resize: 1,
  raw: 2,
  jpeg: 3,
  copy: 4,
  cursor: 5,
  bell: 6,
  text: 7,
  closed: 9
} as const

const HEADER = 9

export interface Rect {
  x: number
  y: number
  w: number
  h: number
}

export type VncFrame =
  | { kind: 'resize'; w: number; h: number }
  | { kind: 'raw'; rect: Rect; pixels: Uint8ClampedArray<ArrayBuffer> }
  | { kind: 'jpeg'; rect: Rect; bytes: Uint8Array<ArrayBuffer> }
  | { kind: 'copy'; dst: Rect; src: { x: number; y: number } }
  | { kind: 'cursor'; rect: Rect; pixels: Uint8ClampedArray<ArrayBuffer> }
  | { kind: 'bell' }
  | { kind: 'text'; text: string }
  | { kind: 'closed'; reason: string; needsPassword: boolean }

/**
 * Разбирает один пакет. `null` — пакет неизвестного или битого вида: рисовать по нему
 * нечего, но и падать нельзя, иначе одна незнакомая кодировка убивает весь экран.
 */
export function parseFrame(buf: ArrayBuffer): VncFrame | null {
  if (buf.byteLength < HEADER) return null
  const v = new DataView(buf)
  const kind = v.getUint8(0)
  const rect: Rect = { x: v.getUint16(1), y: v.getUint16(3), w: v.getUint16(5), h: v.getUint16(7) }

  switch (kind) {
    case VNC_KIND.resize:
      return { kind: 'resize', w: rect.w, h: rect.h }

    case VNC_KIND.raw:
    case VNC_KIND.cursor: {
      const pixels = new Uint8ClampedArray(buf, HEADER)
      // Данных должно хватать ровно на прямоугольник: короткий буфер — это рассинхрон
      // формата, и лучше пропустить кадр, чем нарисовать сдвинутый мусор.
      if (pixels.length < rect.w * rect.h * 4) return null
      return kind === VNC_KIND.raw
        ? { kind: 'raw', rect, pixels }
        : { kind: 'cursor', rect, pixels }
    }

    case VNC_KIND.jpeg:
      return { kind: 'jpeg', rect, bytes: new Uint8Array(buf, HEADER) }

    case VNC_KIND.copy: {
      if (buf.byteLength < HEADER + 4) return null
      return {
        kind: 'copy',
        dst: rect,
        src: { x: v.getUint16(HEADER), y: v.getUint16(HEADER + 2) }
      }
    }

    case VNC_KIND.bell:
      return { kind: 'bell' }

    case VNC_KIND.text:
      return { kind: 'text', text: decodeText(buf) }

    case VNC_KIND.closed:
      // Признак «дело в пароле» приходит флагом в поле x. По тексту его определять нельзя:
      // сообщение идёт от сервера, на его языке и в его формулировке.
      return { kind: 'closed', reason: decodeText(buf), needsPassword: rect.x === 1 }

    default:
      return null
  }
}

function decodeText(buf: ArrayBuffer): string {
  return new TextDecoder().decode(new Uint8Array(buf, HEADER))
}
