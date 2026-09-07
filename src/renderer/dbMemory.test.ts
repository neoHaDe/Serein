import { describe, it, expect, beforeEach } from 'vitest'
import { forget, isGone, recall, remember, update, type DbMemory } from './dbMemory'

const memory = (id = 'db-1'): DbMemory => ({
  connectionId: id,
  info: { kind: 'mysql', host: '172.18.0.7', port: 3306 },
  form: { kind: 'mysql', host: '172.18.0.7', port: '3306', user: 'nextcloud', database: 'nextcloud' },
  text: 'SELECT 1',
  result: null
})

describe('память панели баз', () => {
  beforeEach(() => {
    forget('сессия-1')
    forget('сессия-2')
  })

  it('возвращает то же, что запомнили', () => {
    remember('сессия-1', memory())
    expect(recall('сессия-1')?.connectionId).toBe('db-1')
  })

  it('у каждой сессии своя память', () => {
    // Иначе вторая вкладка показала бы соединение первой.
    remember('сессия-1', memory('db-1'))
    remember('сессия-2', memory('db-2'))
    expect(recall('сессия-1')?.connectionId).toBe('db-1')
    expect(recall('сессия-2')?.connectionId).toBe('db-2')
  })

  it('о незнакомой сессии не выдумывает', () => {
    expect(recall('такой-нет')).toBeNull()
  })

  it('пароль не хранится вовсе', () => {
    // Пока соединение живо, он не нужен; держать его дольше формы - плата без выгоды.
    remember('сессия-1', memory())
    expect(Object.keys(recall('сессия-1')!.form)).not.toContain('password')
  })

  it('частичное обновление не трогает остальное', () => {
    remember('сессия-1', memory())
    update('сессия-1', { text: 'SELECT 2' })
    const m = recall('сессия-1')!
    expect(m.text).toBe('SELECT 2')
    expect(m.connectionId).toBe('db-1')
    expect(m.form.user).toBe('nextcloud')
  })

  it('обновление несуществующей записи ничего не создаёт', () => {
    // Иначе после закрытия соединения в памяти появился бы огрызок без connectionId.
    update('сессия-1', { text: 'SELECT 3' })
    expect(recall('сессия-1')).toBeNull()
  })

  it('забытое не возвращается', () => {
    remember('сессия-1', memory())
    forget('сессия-1')
    expect(recall('сессия-1')).toBeNull()
  })
})

describe('признак умершего соединения', () => {
  it('узнаёт ответ приложения о закрытом соединении', () => {
    // Сессия могла закрыться, пока панель была на другой вкладке.
    expect(isGone('Соединение с базой закрыто')).toBe(true)
    expect(isGone('соединение с базой закрыто')).toBe(true)
  })

  it('обычную ошибку запроса за смерть соединения не принимает', () => {
    // Иначе опечатка в SQL выкидывала бы человека обратно на форму подключения.
    expect(isGone("1146: Table 'probe.нет' doesn't exist")).toBe(false)
    expect(isGone('syntax error at or near "SELEKT"')).toBe(false)
  })
})
