import { describe, it, expect } from 'vitest'
import { matchesQuery, nextSort, parseBytes, parsePercent, sortRows } from './tableSort'

describe('сортировка таблиц', () => {
  it('по числам в обе стороны, а пустые значения всегда внизу', () => {
    const rows = [
      { n: 'a', v: 5 },
      { n: 'b', v: null },
      { n: 'c', v: 50 },
      { n: 'd', v: 0.5 }
    ]
    const cell = (r: (typeof rows)[number]): number | null => r.v
    expect(sortRows(rows, { key: 'v', dir: 'desc' }, cell).map((r) => r.n)).toEqual(['c', 'a', 'd', 'b'])
    // По возрастанию список не должен начинаться с прочерков.
    expect(sortRows(rows, { key: 'v', dir: 'asc' }, cell).map((r) => r.n)).toEqual(['d', 'a', 'c', 'b'])
  })

  it('по строкам - с числами внутри имени, равные сохраняют прежний порядок', () => {
    const names = [{ n: 'nginx-10' }, { n: 'nginx-9' }, { n: 'Nginx-1' }]
    expect(sortRows(names, { key: 'n', dir: 'asc' }, (r) => r.n).map((r) => r.n)).toEqual([
      'Nginx-1',
      'nginx-9',
      'nginx-10'
    ])
    const same = [{ id: 1 }, { id: 2 }, { id: 3 }]
    expect(sortRows(same, { key: 'x', dir: 'desc' }, () => 7).map((r) => r.id)).toEqual([1, 2, 3])
  })

  it('щелчок по тому же столбцу меняет направление, по другому - начинает заново', () => {
    expect(nextSort({ key: 'cpu', dir: 'desc' }, 'cpu', 'desc')).toEqual({ key: 'cpu', dir: 'asc' })
    expect(nextSort({ key: 'cpu', dir: 'asc' }, 'name', 'asc')).toEqual({ key: 'name', dir: 'asc' })
    expect(nextSort({ key: 'name', dir: 'asc' }, 'mem', 'desc')).toEqual({ key: 'mem', dir: 'desc' })
  })

  it('поиск находит все слова запроса, каждое в любом поле', () => {
    const row = [1234, 'root', 'S', 'nginx: worker process']
    expect(matchesQuery(row, 'root nginx')).toBe(true)
    expect(matchesQuery(row, '  NGINX   worker ')).toBe(true)
    expect(matchesQuery(row, 'root postgres')).toBe(false)
    expect(matchesQuery(row, '123')).toBe(true)
    expect(matchesQuery(row, '')).toBe(true)
    expect(matchesQuery([null, undefined, 'web'], 'null')).toBe(false)
  })

  it('разбирает доли и объёмы из docker stats', () => {
    expect(parsePercent('13.45%')).toBe(13.45)
    expect(parsePercent('--')).toBeNull()
    expect(parsePercent(undefined)).toBeNull()
    expect(parseBytes('482MiB / 2GiB')).toBe(482 * 1024 ** 2)
    expect(parseBytes('1.5kB / 2kB')).toBe(1500)
    expect(parseBytes('0B / 0B')).toBe(0)
    expect(parseBytes('')).toBeNull()
  })
})
