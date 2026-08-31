import { describe, it, expect } from 'vitest'
import { EMPTY_SELECTION, clickSelect, pruneSelection, targetsFor } from './selection'

const order = ['a', 'b', 'c', 'd']

describe('выделение серверов', () => {
  it('обычный клик оставляет одну строку', () => {
    const s = clickSelect({ ids: ['a', 'b'], anchor: 'a' }, 'c', order)
    expect(s.ids).toEqual(['c'])
    expect(s.anchor).toBe('c')
  })

  it('Ctrl добавляет и убирает по одному', () => {
    let s = clickSelect(EMPTY_SELECTION, 'a', order)
    s = clickSelect(s, 'c', order, { ctrl: true })
    expect(s.ids).toEqual(['a', 'c'])
    s = clickSelect(s, 'a', order, { ctrl: true })
    expect(s.ids).toEqual(['c'])
  })

  it('снятая Ctrl строка перестаёт быть якорем', () => {
    // Иначе следующий Shift потянется от строки, которая уже не выделена.
    let s = clickSelect(EMPTY_SELECTION, 'b', order)
    s = clickSelect(s, 'b', order, { ctrl: true })
    expect(s.ids).toEqual([])
    expect(s.anchor).toBeUndefined()
  })

  it('Shift берёт отрезок от якоря в любую сторону', () => {
    const s = clickSelect({ ids: ['c'], anchor: 'c' }, 'a', order, { shift: true })
    expect(s.ids).toEqual(['a', 'b', 'c'])
    const back = clickSelect(s, 'd', order, { shift: true })
    expect(back.ids).toEqual(['c', 'd'])
  })

  it('Shift без якоря ведёт себя как обычный клик', () => {
    expect(clickSelect(EMPTY_SELECTION, 'c', order, { shift: true }).ids).toEqual(['c'])
  })

  it('Shift считает отрезок по видимому порядку', () => {
    // Список отфильтрован: пользователь ждёт отрезок из того, что перед глазами.
    const visible = ['a', 'd']
    const s = clickSelect({ ids: ['a'], anchor: 'a' }, 'd', visible, { shift: true })
    expect(s.ids).toEqual(['a', 'd'])
  })

  it('исчезнувшие серверы выпадают из выделения', () => {
    const s = pruneSelection({ ids: ['a', 'b'], anchor: 'b' }, ['a'])
    expect(s.ids).toEqual(['a'])
    expect(s.anchor).toBeUndefined()
  })

  it('неизменное выделение возвращается тем же объектом', () => {
    // Чтобы не дёргать перерисовку списка на каждом обновлении серверов.
    const s = { ids: ['a'], anchor: 'a' }
    expect(pruneSelection(s, order)).toBe(s)
  })
})

describe('к чему применяется действие', () => {
  it('одиночный выбор — к строке, по которой щёлкнули', () => {
    expect(targetsFor({ ids: ['a'], anchor: 'a' }, 'b')).toEqual(['b'])
    expect(targetsFor(EMPTY_SELECTION, 'b')).toEqual(['b'])
  })

  it('групповое действие — ко всему выделению', () => {
    expect(targetsFor({ ids: ['a', 'b'], anchor: 'a' }, 'b')).toEqual(['a', 'b'])
  })

  it('клик вне выделения не тянет за собой чужие строки', () => {
    expect(targetsFor({ ids: ['a', 'b'], anchor: 'a' }, 'd')).toEqual(['d'])
  })
})
