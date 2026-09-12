import { describe, expect, it } from 'vitest'
import {
  allowClipboardFromServer,
  CLIPBOARD_FROM_SERVER_EVERY_MS,
  CLIPBOARD_FROM_SERVER_MAX
} from './clipboardPolicy'

const ask = (over: Partial<Parameters<typeof allowClipboardFromServer>[0]> = {}): boolean =>
  allowClipboardFromServer({ focused: true, length: 10, sinceLastMs: 10_000, ...over })

describe('буфер обмена от сервера', () => {
  it('пропускает обычное копирование из активного терминала', () => {
    expect(ask()).toBe(true)
  })

  it('не даёт фоновой вкладке менять буфер', () => {
    // Самый неприятный случай: вы копируете у себя, а соседняя сессия подменяет буфер, и
    // узнаёте вы об этом уже после вставки.
    expect(ask({ focused: false })).toBe(false)
  })

  it('отказывает слишком большому тексту', () => {
    expect(ask({ length: CLIPBOARD_FROM_SERVER_MAX })).toBe(true)
    expect(ask({ length: CLIPBOARD_FROM_SERVER_MAX + 1 })).toBe(false)
    expect(ask({ length: 0 })).toBe(false)
  })

  it('не даёт занимать буфер потоком последовательностей', () => {
    expect(ask({ sinceLastMs: CLIPBOARD_FROM_SERVER_EVERY_MS })).toBe(true)
    expect(ask({ sinceLastMs: CLIPBOARD_FROM_SERVER_EVERY_MS - 1 })).toBe(false)
  })
})
