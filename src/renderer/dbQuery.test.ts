import { describe, it, expect } from 'vitest'
import { cellText, isNull, needsConfirm, summarize } from './dbQuery'

const result = (rows: number, affected = 0, ms = 12): Parameters<typeof summarize>[0] => ({
  columns: ['a'],
  rows: Array.from({ length: rows }, () => ({ a: 1 })),
  affected,
  ms
})

describe('итог запроса', () => {
  it('склоняет слово «строка» по-русски', () => {
    expect(summarize(result(1))).toBe('1 строка · 12 мс')
    expect(summarize(result(3))).toBe('3 строки · 12 мс')
    expect(summarize(result(5))).toBe('5 строк · 12 мс')
    expect(summarize(result(21))).toBe('21 строка · 12 мс')
  })

  it('числа от 11 до 14 — исключение, а не правило', () => {
    // Наивное «n % 10» даёт «11 строка» и «12 строки».
    expect(summarize(result(11))).toBe('11 строк · 12 мс')
    expect(summarize(result(12))).toBe('12 строк · 12 мс')
    expect(summarize(result(14))).toBe('14 строк · 12 мс')
  })

  it('запрос без выборки сообщает про изменённые строки', () => {
    expect(summarize(result(0, 2))).toBe('изменено 2 строки · 12 мс')
    expect(summarize(result(0, 0))).toBe('выполнено · 12 мс')
  })
})

describe('предупреждение перед выполнением', () => {
  it('обычные запросы не требуют подтверждения', () => {
    expect(needsConfirm('SELECT * FROM users')).toBeNull()
    expect(needsConfirm('DELETE FROM users WHERE id = 1')).toBeNull()
    expect(needsConfirm('UPDATE users SET name = \'x\' WHERE id = 1')).toBeNull()
    expect(needsConfirm('')).toBeNull()
  })

  it('DELETE и UPDATE без условия предупреждают', () => {
    expect(needsConfirm('DELETE FROM users')).toMatch(/все строки/)
    expect(needsConfirm('update users set active = false')).toMatch(/все строки/)
  })

  it('DROP и TRUNCATE предупреждают всегда', () => {
    expect(needsConfirm('DROP TABLE users')).toMatch(/необратимо/)
    expect(needsConfirm('TRUNCATE users')).toMatch(/целиком/)
  })

  it('условие в комментарии не считается условием', () => {
    // Иначе `DELETE FROM t -- WHERE допишу` пройдёт как безопасный запрос.
    expect(needsConfirm('DELETE FROM users -- WHERE id = 1')).toMatch(/все строки/)
    expect(needsConfirm('DELETE FROM users /* WHERE id = 1 */')).toMatch(/все строки/)
  })

  it('слово WHERE внутри строки не считается условием', () => {
    expect(needsConfirm("DELETE FROM logs -- 'where'")).toMatch(/все строки/)
  })

  it('команды Redis, стирающие базу, тоже предупреждают', () => {
    expect(needsConfirm('FLUSHALL')).toMatch(/целиком/)
    expect(needsConfirm('flushdb')).toMatch(/целиком/)
  })
})

describe('показ значений', () => {
  it('NULL отличается от пустой строки', () => {
    // В таблице это разные вещи: пустая строка — значение, NULL — его отсутствие.
    expect(cellText(null)).toBe('NULL')
    expect(cellText('')).toBe('')
    expect(isNull(null)).toBe(true)
    expect(isNull('')).toBe(false)
  })

  it('нестроковые значения показываются как есть', () => {
    expect(cellText(42)).toBe('42')
    expect(cellText(true)).toBe('true')
  })
})
