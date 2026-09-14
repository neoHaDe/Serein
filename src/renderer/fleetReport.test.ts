import { describe, it, expect } from 'vitest'
import type { MultiExecResult } from '../shared/types'
import {
  clampConcurrency,
  clampTimeout,
  fleetReport,
  mergeResults,
  retryIds,
  summarizeFleet,
  summaryText
} from './fleetReport'

const done = (id: string, code: number, stdout = ''): MultiExecResult => ({
  serverId: id,
  name: id,
  state: 'done',
  code,
  stdout,
  ms: 10
})
const failed = (id: string): MultiExecResult => ({ serverId: id, name: id, state: 'failed', error: 'сеть' })
const skipped = (id: string): MultiExecResult => ({ serverId: id, name: id, state: 'skipped', error: 'ключ' })

describe('сводка прогона', () => {
  it('разводит успех, коды возврата, недошедших и пропущенных', () => {
    const s = summarizeFleet([done('a', 0), done('b', 2), done('c', 1), done('d', 1), failed('e'), skipped('f')])
    expect(s).toEqual({ ok: 1, codes: [[1, 2], [2, 1]], failed: 1, skipped: 1 })
    expect(summaryText(s)).toBe('успешно 1 · код 1: 2 · код 2: 1 · не дошли 1 · пропущено 1')
    expect(summaryText(summarizeFleet([]))).toBe('нет результатов')
  })
})

describe('повтор упавших', () => {
  it('повторяет недошедших и ненулевые коды, но не пропущенных', () => {
    // Пропущенный из-за неподтверждённого ключа пропустится и при повторе.
    expect(retryIds([done('a', 0), done('b', 3), failed('c'), skipped('d')])).toEqual(['b', 'c'])
  })

  it('повторённый хост заменяет свою строку, а не дописывается второй', () => {
    const merged = mergeResults([done('a', 0), failed('b'), done('c', 1)], [done('b', 0), done('c', 0)])
    expect(merged.map((r) => [r.serverId, r.state, r.code])).toEqual([
      ['a', 'done', 0],
      ['b', 'done', 0],
      ['c', 'done', 0]
    ])
    expect(mergeResults([done('a', 0)], [done('z', 0)]).map((r) => r.serverId)).toEqual(['a', 'z'])
  })
})

describe('параметры прогона', () => {
  it('держатся в пределах бэкенда', () => {
    expect(clampConcurrency(0)).toBe(1)
    expect(clampConcurrency(1000)).toBe(64)
    expect(clampConcurrency(Number.NaN)).toBe(4)
    expect(clampTimeout(-5)).toBe(1)
    expect(clampTimeout(99999)).toBe(3600)
  })
})

describe('отчёт', () => {
  it('начинается со сводки и ставит упавших первыми', () => {
    const text = fleetReport('uptime', [done('ok', 0, ' up 3 days'), failed('bad'), done('code', 5)], new Date(2026, 8, 14))
    expect(text).toContain('Команда: uptime')
    expect(text).toContain('Итог: успешно 1 · код 5: 1 · не дошли 1')
    const order = ['=== bad', '=== code', '=== ok'].map((h) => text.indexOf(h))
    expect(order.every((pos) => pos >= 0)).toBe(true)
    expect([...order].sort((a, b) => a - b)).toEqual(order)
    expect(text).toContain('up 3 days')
  })
})
