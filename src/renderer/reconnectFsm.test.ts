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
