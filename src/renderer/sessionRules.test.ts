import { describe, expect, it } from 'vitest'
import { shouldAutoReconnect } from './sessionRules'

const drop = { reason: 'drop' as const }
const live = { kind: 'ssh' as const, status: 'connected' as const }
const on = { autoReconnect: true }

describe('shouldAutoReconnect', () => {
  it('переподключает живое SSH-соединение при обрыве', () => {
    expect(shouldAutoReconnect(drop, live, on)).toBe(true)
  })

  it('не воскрешает сессию, закрытую пользователем', () => {
    // Самый заметный из возможных промахов: закрытая вкладка открывалась бы снова.
    expect(shouldAutoReconnect({ reason: 'user' }, live, on)).toBe(false)
    expect(shouldAutoReconnect({}, live, on)).toBe(false)
  })

  it('не трогает не-SSH', () => {
    // У локального терминала просто закончился процесс, у COM-порта и telnet нет
    // понятия «то же соединение».
    for (const kind of ['local', 'serial', 'telnet', 'raw'] as const) {
      expect(shouldAutoReconnect(drop, { kind, status: 'connected' }, on)).toBe(false)
    }
  })

  it('уважает выключенную настройку', () => {
    expect(shouldAutoReconnect(drop, live, { autoReconnect: false })).toBe(false)
    expect(shouldAutoReconnect(drop, live, {})).toBe(false)
  })

  it('переподключает и то, что было на полпути', () => {
    // Обрыв во время подключения или уже идущего переподключения — тот же случай.
    expect(shouldAutoReconnect(drop, { kind: 'ssh', status: 'connecting' }, on)).toBe(true)
    expect(shouldAutoReconnect(drop, { kind: 'ssh', status: 'reconnecting' }, on)).toBe(true)
  })

  it('не повторяет хвост прошлой неудачи', () => {
    // Панель уже в ошибке или закрыта — это не обрыв рабочего соединения.
    expect(shouldAutoReconnect(drop, { kind: 'ssh', status: 'error' }, on)).toBe(false)
    expect(shouldAutoReconnect(drop, { kind: 'ssh', status: 'closed' }, on)).toBe(false)
  })
})
