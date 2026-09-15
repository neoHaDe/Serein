import { describe, it, expect } from 'vitest'
import { lockedText, settingLabel } from './policyText'

describe('плашка политики', () => {
  it('называет настройки словами, незнакомый ключ показывает как есть', () => {
    expect(lockedText(['actionLog', 'offline'])).toBe('журнал действий, закрытый контур')
    expect(settingLabel('somethingNew')).toBe('somethingNew')
    expect(lockedText([])).toBe('')
  })
})
