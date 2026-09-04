import { describe, it, expect } from 'vitest'
import { buttonMask, keysymFor, wheelMask } from './vncKeys'

const key = (k: string, code = ''): Pick<KeyboardEvent, 'key' | 'code'> => ({ key: k, code })

describe('перевод клавиш в keysym', () => {
  it('ASCII совпадает с кодом символа', () => {
    expect(keysymFor(key('a', 'KeyA'))).toBe(0x61)
    expect(keysymFor(key('A', 'KeyA'))).toBe(0x41)
    expect(keysymFor(key('1', 'Digit1'))).toBe(0x31)
    expect(keysymFor(key(' ', 'Space'))).toBe(0x20)
  })

  it('кириллица уезжает своими историческими кодами, а не юникодом', () => {
    // X11 держит кириллицу в диапазоне, унаследованном от КОИ-8. Юникод-форму
    // (0x01000000 + код) понимают не все серверы — старые её игнорируют, и русский текст
    // не набирается вовсе.
    expect(keysymFor(key('а', 'KeyF'))).toBe(0x6c1)
    expect(keysymFor(key('Ж', 'Semicolon'))).toBe(0x6f6)
    expect(keysymFor(key('ё', 'Backquote'))).toBe(0x6a3)
    expect(keysymFor(key('я', 'KeyZ'))).toBe(0x6d1)
  })

  it('раскладка важнее физической клавиши', () => {
    // Наивная реализация на `code` отправила бы латинскую букву: физически это KeyF,
    // но пользователь набрал «а».
    expect(keysymFor(key('а', 'KeyF'))).not.toBe(0x66)
  })

  it('юникод за пределами кириллицы и Latin-1 идёт общей формой', () => {
    expect(keysymFor(key('日', 'KeyA'))).toBe(0x01000000 + 0x65e5)
  })

  it('Latin-1 остаётся однобайтовым', () => {
    expect(keysymFor(key('ü', 'BracketLeft'))).toBe(0xfc)
  })

  it('функциональные клавиши считаются от F1', () => {
    expect(keysymFor(key('F1', 'F1'))).toBe(0xffbe)
    expect(keysymFor(key('F12', 'F12'))).toBe(0xffc9)
  })

  it('левый и правый модификаторы различаются', () => {
    // `key` у них одинаковый — различить можно только по `code`.
    expect(keysymFor(key('Shift', 'ShiftLeft'))).toBe(0xffe1)
    expect(keysymFor(key('Shift', 'ShiftRight'))).toBe(0xffe2)
    expect(keysymFor(key('Control', 'ControlLeft'))).toBe(0xffe3)
  })

  it('управляющие клавиши берутся из таблицы', () => {
    expect(keysymFor(key('Enter', 'Enter'))).toBe(0xff0d)
    expect(keysymFor(key('Backspace', 'Backspace'))).toBe(0xff08)
    expect(keysymFor(key('Delete', 'Delete'))).toBe(0xffff)
    expect(keysymFor(key('ArrowUp', 'ArrowUp'))).toBe(0xff52)
  })

  it('неизвестная многосимвольная клавиша не отправляется', () => {
    // Лучше не послать ничего, чем послать случайный keysym.
    expect(keysymFor(key('Unidentified', 'Unknown'))).toBeNull()
    expect(keysymFor(key('BrightnessUp', 'Xyz'))).toBeNull()
  })
})

describe('мышь', () => {
  it('порядок кнопок в RFB не такой, как в браузере', () => {
    // В браузере 2 — правая, в RFB правая это 4, а 2 — средняя. Перепутать легко.
    expect(buttonMask(1)).toBe(1)
    expect(buttonMask(2)).toBe(4)
    expect(buttonMask(4)).toBe(2)
    expect(buttonMask(3)).toBe(5)
    expect(buttonMask(0)).toBe(0)
  })

  it('колесо — это кнопки 4 и 5', () => {
    expect(wheelMask(-1)).toBe(8)
    expect(wheelMask(1)).toBe(16)
    expect(wheelMask(0)).toBe(0)
    expect(wheelMask(0, -1)).toBe(32)
    expect(wheelMask(0, 1)).toBe(64)
  })
})
