import { describe, it, expect } from 'vitest'
import { parseFrame, VNC_KIND } from './vncFrames'

/** Собирает пакет так же, как это делает Rust: заголовок из девяти байт плюс тело. */
function pack(kind: number, x: number, y: number, w: number, h: number, body: number[] = []): ArrayBuffer {
  const buf = new ArrayBuffer(9 + body.length)
  const v = new DataView(buf)
  v.setUint8(0, kind)
  v.setUint16(1, x)
  v.setUint16(3, y)
  v.setUint16(5, w)
  v.setUint16(7, h)
  new Uint8Array(buf, 9).set(body)
  return buf
}

describe('разбор пакетов рабочего стола', () => {
  it('смена разрешения', () => {
    const f = parseFrame(pack(VNC_KIND.resize, 0, 0, 1920, 1080))
    expect(f).toEqual({ kind: 'resize', w: 1920, h: 1080 })
  })

  it('сырой прямоугольник несёт пиксели', () => {
    const px = [1, 2, 3, 255, 4, 5, 6, 255]
    const f = parseFrame(pack(VNC_KIND.raw, 10, 20, 2, 1, px))
    expect(f?.kind).toBe('raw')
    if (f?.kind !== 'raw') throw new Error('ожидали raw')
    expect(f.rect).toEqual({ x: 10, y: 20, w: 2, h: 1 })
    expect([...f.pixels]).toEqual(px)
  })

  it('короткий буфер пикселей отбрасывается', () => {
    // Рассинхрон формата с бэкендом. Нарисовать такой пакет — значит сдвинуть всю
    // картинку и потом искать причину глазами.
    expect(parseFrame(pack(VNC_KIND.raw, 0, 0, 4, 4, [1, 2, 3, 255]))).toBeNull()
  })

  it('копирование области помнит и источник, и приёмник', () => {
    const buf = new ArrayBuffer(13)
    const v = new DataView(buf)
    v.setUint8(0, VNC_KIND.copy)
    v.setUint16(1, 100)
    v.setUint16(3, 200)
    v.setUint16(5, 50)
    v.setUint16(7, 60)
    v.setUint16(9, 5)
    v.setUint16(11, 7)
    expect(parseFrame(buf)).toEqual({
      kind: 'copy',
      dst: { x: 100, y: 200, w: 50, h: 60 },
      src: { x: 5, y: 7 }
    })
  })

  it('копирование без координат источника отбрасывается', () => {
    expect(parseFrame(pack(VNC_KIND.copy, 1, 1, 1, 1))).toBeNull()
  })

  it('текст буфера обмена декодируется как UTF-8', () => {
    const bytes = [...new TextEncoder().encode('привет')]
    expect(parseFrame(pack(VNC_KIND.text, 0, 0, 0, 0, bytes))).toEqual({
      kind: 'text',
      text: 'привет'
    })
  })

  it('закрытие несёт причину', () => {
    const bytes = [...new TextEncoder().encode('Обрыв связи')]
    expect(parseFrame(pack(VNC_KIND.closed, 0, 0, 0, 0, bytes))).toEqual({
      kind: 'closed',
      reason: 'Обрыв связи',
      needsPassword: false
    })
  })

  it('отказ по паролю приходит флагом, а не текстом', () => {
    // Текст сообщения даёт сервер — он на его языке и в его формулировке. Строить на нём
    // показ формы ввода значит ломаться от чужой правки.
    const bytes = [...new TextEncoder().encode('password check failed')]
    expect(parseFrame(pack(VNC_KIND.closed, 1, 0, 0, 0, bytes))).toEqual({
      kind: 'closed',
      reason: 'password check failed',
      needsPassword: true
    })
  })

  it('JPEG отдаётся как есть — его декодирует браузер', () => {
    const f = parseFrame(pack(VNC_KIND.jpeg, 0, 0, 8, 8, [0xff, 0xd8, 0xff]))
    if (f?.kind !== 'jpeg') throw new Error('ожидали jpeg')
    expect([...f.bytes]).toEqual([0xff, 0xd8, 0xff])
  })

  it('незнакомый тип и обрезанный пакет не роняют разбор', () => {
    // Одна незнакомая кодировка не должна убивать весь экран.
    expect(parseFrame(pack(99, 0, 0, 0, 0))).toBeNull()
    expect(parseFrame(new ArrayBuffer(4))).toBeNull()
  })
})
