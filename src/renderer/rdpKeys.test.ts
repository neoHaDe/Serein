import { describe, it, expect } from 'vitest'
import { buttonMask, isModifier, scancodeFor } from './rdpKeys'

/*
 * Таблица скан-кодов проверяется тестом, а не глазами, по той же причине, что и разбор
 * кадров: ошибка здесь не падает, а тихо печатает не ту букву или шлёт не ту стрелку,
 * и ловится это уже руками на живом сервере.
 */

describe('скан-коды RDP', () => {
  it('буквы берутся по месту на клавиатуре, а не по символу', () => {
    // В этом всё отличие от VNC: `code` описывает физическую клавишу, раскладку сервер
    // применит свою. Поэтому KeyA - это 0x1E независимо от того, «а» там или «a».
    expect(scancodeFor('KeyA')).toBe(0x1e)
    expect(scancodeFor('KeyZ')).toBe(0x2c)
    expect(scancodeFor('Digit1')).toBe(0x02)
  })

  it('расширенные помечаются префиксом', () => {
    // Помощник считает расширенной ту клавишу, у которой старшие биты 0xE000.
    expect(scancodeFor('ArrowUp')).toBe(0xe048)
    expect(scancodeFor('Delete')).toBe(0xe053)
    expect(scancodeFor('ControlRight')).toBe(0xe01d)
  })

  it('левые и правые различаются', () => {
    // Ctrl слева обычный, справа расширенный - разные клавиши, и сервер это видит.
    expect(scancodeFor('ControlLeft')).toBe(0x1d)
    expect(scancodeFor('ControlRight')).toBe(0xe01d)
    expect(scancodeFor('AltLeft')).toBe(0x38)
    expect(scancodeFor('AltRight')).toBe(0xe038)
  })

  it('Enter на цифровой клавиатуре не тот же, что основной', () => {
    // Код совпадает, префикс - нет. Перепутав, получим Enter не там, где нажали.
    expect(scancodeFor('Enter')).toBe(0x1c)
    expect(scancodeFor('NumpadEnter')).toBe(0xe01c)
  })

  it('неизвестная клавиша даёт null, а не что-то похожее', () => {
    // Отправить вместо неизвестной клавиши соседнюю хуже, чем не отправить ничего:
    // человек увидит не то, что нажал, и не поймёт почему.
    expect(scancodeFor('LaunchMail')).toBeNull()
    expect(scancodeFor('')).toBeNull()
  })
})

describe('кнопки мыши', () => {
  it('маска совпадает с браузерной', () => {
    expect(buttonMask(1)).toBe(1)
    expect(buttonMask(2)).toBe(2)
    expect(buttonMask(4)).toBe(4)
  })

  it('лишние биты отбрасываются', () => {
    // У мыши бывают четвёртая и пятая кнопки; RDP их в этой раскладке не ждёт, и
    // передавать их как одну из первых трёх было бы враньём.
    expect(buttonMask(0b11111)).toBe(0b111)
  })

  it('авто-повтор модификатора отличается от обычной клавиши', () => {
    // Библиотека ввода RDP на повторное нажатие добавляет отпускание и новое нажатие.
    // Для буквы это верно, а для Alt посреди Alt+Shift читается сервером как второе
    // переключение раскладки: она меняется и сразу возвращается обратно.
    for (const code of ['AltLeft', 'AltRight', 'ShiftLeft', 'ControlRight', 'MetaLeft', 'CapsLock']) {
      expect(isModifier(code)).toBe(true)
    }
    for (const code of ['KeyA', 'Space', 'F5', 'Digit1', 'Enter']) {
      expect(isModifier(code)).toBe(false)
    }
  })
})
