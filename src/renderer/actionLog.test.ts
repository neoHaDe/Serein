import { describe, it, expect } from 'vitest'
import { actionLabel, detailText, matchesEntry, serverText, timeText, verifyText, type ActionLogEntry } from './actionLog'

const entry = (over: Partial<ActionLogEntry>): ActionLogEntry => ({
  seq: 1,
  t: '2026-09-15T10:20:30.000Z',
  action: 'ssh.connect',
  ok: true,
  ...over
})

describe('журнал действий в окне', () => {
  it('строка терминала: набранное, дописанное сервером и скрытое', () => {
    expect(detailText(entry({ action: 'terminal.line', detail: { line: 'sudo reboot', edited: false } }))).toBe('sudo reboot')
    expect(detailText(entry({ action: 'terminal.line', detail: { line: 'systemctl rest', edited: true } }))).toContain(
      'дописал или заменил сервер'
    )
    expect(detailText(entry({ action: 'terminal.line', detail: { line: '', edited: true } }))).toContain('из истории')
    expect(
      detailText(entry({ action: 'terminal.line', detail: { hidden: 'набрано после запроса пароля - не записано' } }))
    ).toContain('пароля')
  })

  it('главное в подробностях - первым, переименование стрелкой', () => {
    expect(detailText(entry({ action: 'fleet.exec', detail: { state: 'done', code: 0, command: 'uptime' } }))).toBe(
      'uptime · state: done · code: 0'
    )
    expect(detailText(entry({ action: 'file.rename', detail: { from: '/a', to: '/b' } }))).toBe('/a → /b')
    expect(detailText(entry({ action: 'file.upload', detail: { remoteDir: '/srv', paths: ['a', 'b'] } }))).toBe(
      'remoteDir: /srv · paths: 2'
    )
  })

  it('сервер словами, порт 22 не показывается', () => {
    expect(serverText(entry({ server: { id: 's1', name: 'prod', host: '10.0.0.1', port: 22, user: 'root' } }))).toBe(
      'prod (root@10.0.0.1)'
    )
    expect(serverText(entry({ server: { id: 's1', host: 'db', port: 2222 } }))).toBe('db:2222')
    expect(serverText(entry({ server: null }))).toBe('')
  })

  it('поиск по серверу, действию словами и подробностям', () => {
    const e = entry({
      action: 'terminal.line',
      server: { id: 's', name: 'prod' },
      detail: { line: 'rm -rf /var/cache', edited: false }
    })
    expect(matchesEntry(e, 'prod rm')).toBe(true)
    expect(matchesEntry(e, 'строка cache')).toBe(true)
    expect(matchesEntry(e, 'stage')).toBe(false)
  })

  it('подписи, время и итог проверки', () => {
    expect(actionLabel('db.query')).toBe('Запрос к базе')
    expect(actionLabel('unknown.thing')).toBe('unknown.thing')
    expect(timeText('не время')).toBe('не время')
    expect(timeText('2026-09-15T10:20:30.000Z')).toMatch(/^2026-09-15 \d\d:20:30$/)
    expect(verifyText({ ok: true, count: 12, files: 1 })).toBe('Цепочка цела: 12 записей в 1 файлах.')
    expect(verifyText({ ok: false, count: 4, file: 'actions-2026-09.jsonl', line: 5, reason: 'хеш не сходится' })).toContain(
      'строка 5'
    )
  })
})
