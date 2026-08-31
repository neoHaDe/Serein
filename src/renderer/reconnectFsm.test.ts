import { describe, expect, it } from 'vitest'
import { sessionClosedMessage, shouldScheduleReconnect } from './reconnectFsm'

const sshLeaf = { kind: 'ssh' as const, status: 'connected' as const }
const on = { autoReconnect: true }

describe('shouldScheduleReconnect', () => {
  it('не переподключает без галочки', () => {
    expect(
      shouldScheduleReconnect('session_drop', sshLeaf, { autoReconnect: false }, { reason: 'drop' })
    ).toBe(false)
  })

  it('не переподключает локальный терминал', () => {
    expect(
      shouldScheduleReconnect(
        'session_drop',
        { kind: 'local', status: 'connected' },
        on,
        { reason: 'drop' }
      )
    ).toBe(false)
  })

  it('переподключает при обрыве живой SSH-панели', () => {
    expect(
      shouldScheduleReconnect('session_drop', sshLeaf, on, { reason: 'drop' })
    ).toBe(true)
  })

  it('не переподключает после закрытия пользователем', () => {
    expect(
      shouldScheduleReconnect('session_drop', sshLeaf, on, { reason: 'user' })
    ).toBe(false)
  })

  it('переподключает при ошибке первого подключения', () => {
    expect(
      shouldScheduleReconnect('connect_fail', { kind: 'ssh', status: 'connecting' }, on)
    ).toBe(true)
  })

  it('не переподключает connect_fail если панель уже закрыта', () => {
    expect(
      shouldScheduleReconnect('connect_fail', { kind: 'ssh', status: 'closed' }, on)
    ).toBe(false)
  })
})

describe('sessionClosedMessage', () => {
  it('добавляет фазу сбоя', () => {
    expect(
      sessionClosedMessage({ error: 'Соединение разорвано', phase: 'shell' })
    ).toContain('во время сессии')
  })
})

describe('фаза сбоя решает, повторять ли', () => {
  const on = { autoReconnect: true }
  const connecting = { kind: 'ssh' as const, status: 'connecting' as const }

  it('не повторяет неверный пароль', () => {
    // Пять попыток с задержками 1–2–4–8–15 с — это шесть неудачных аутентификаций за
    // полминуты: порог типичного fail2ban и блокировки доменной учётной записи.
    expect(shouldScheduleReconnect('connect_fail', connecting, on, undefined, 'auth')).toBe(false)
  })

  it('не повторяет отклонённый ключ хоста', () => {
    expect(shouldScheduleReconnect('connect_fail', connecting, on, undefined, 'hostkey')).toBe(false)
  })

  it('повторяет сетевые сбои', () => {
    // Тут повтор и правда лечит: хост мог моргнуть.
    expect(shouldScheduleReconnect('connect_fail', connecting, on, undefined, 'connect')).toBe(true)
    expect(shouldScheduleReconnect('connect_fail', connecting, on, undefined, 'jump')).toBe(true)
  })

  it('без фазы ведёт себя как раньше', () => {
    // Старые пути (serial, telnet, неизвестная ошибка) фазу не присылают.
    expect(shouldScheduleReconnect('connect_fail', connecting, on)).toBe(true)
  })
})
