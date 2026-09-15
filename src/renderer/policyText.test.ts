import { describe, it, expect } from 'vitest'
import { lockedText, policySummary, settingLabel } from './policyText'

describe('плашка политики', () => {
  it('называет настройки словами, незнакомый ключ показывает как есть', () => {
    expect(lockedText(['actionLog', 'offline'])).toBe('журнал действий, закрытый контур')
    expect(settingLabel('somethingNew')).toBe('somethingNew')
    expect(lockedText([])).toBe('')
  })

  it('перечисляет все ограничения, пустой список адресов - запрет всех', () => {
    const base = { sources: ['реестр'], locked: [], forbidLegacySshAlgorithms: false, error: null }
    expect(policySummary(base)).toEqual([])
    expect(
      policySummary({
        ...base,
        locked: ['offline'],
        allowedHosts: ['*.corp.local'],
        forbidSavedPasswords: true,
        requireMasterPassword: true,
        forbidLocalTerminal: true,
        forbidSessionRecording: true
      })
    ).toEqual([
      'заданы настройки: закрытый контур',
      'подключаться можно только к: *.corp.local',
      'пароли не сохраняются и спрашиваются при подключении',
      'мастер-пароль обязателен',
      'локальный терминал запрещён',
      'запись сессии в файл запрещена'
    ])
    expect(policySummary({ ...base, allowedHosts: [] })).toEqual(['подключаться нельзя ни к одному серверу'])
  })
})
