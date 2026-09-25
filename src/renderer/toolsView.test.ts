import { describe, expect, it } from 'vitest'
import { describeTool, msText, plural } from './toolsView'

// 2026-09-25 12:00 UTC: от него считаются сроки сертификатов и токенов.
const NOW = Date.UTC(2026, 8, 25, 12)
const DAY = 86_400

describe('ответ утилиты словами', () => {
  it('склоняет по числу', () => {
    expect([1, 2, 5, 11, 21, 22, 112].map((n) => plural(n, 'узел', 'узла', 'узлов'))).toEqual([
      'узел',
      'узла',
      'узлов',
      'узлов',
      'узел',
      'узла',
      'узлов'
    ])
  })

  it('длительность по-русски', () => {
    expect([0, 3, 0.4, 999, 2300].map(msText)).toEqual(['меньше 1 мс', '3 мс', '0,4 мс', '999 мс', '2,3 с'])
  })

  it('порт: открыт и закрыт, с этой машины и с сервера', () => {
    const open = describeTool('port', { ok: true, host: 'web-01', port: 80, latencyMs: 3 })!
    expect(open.verdict).toEqual({ tone: 'ok', text: 'Порт web-01:80 открыт' })
    expect(open.rows).toEqual([['Отклик', '3 мс']])

    const refused = describeTool('port', {
      ok: false,
      host: '10.0.0.5',
      port: 5432,
      error: 'Connection refused (os error 111)'
    })!
    expect(refused.verdict?.tone).toBe('bad')
    expect(refused.rows?.[0]?.[1]).toContain('никто не слушает')

    const server = describeTool('port', {
      ok: false,
      host: 'db',
      port: 5432,
      from: 'server',
      tool: 'nc',
      error: 'Порт закрыт или недоступен с сервера'
    })!
    expect(server.rows).toContainEqual(['Откуда', 'с сервера, через nc'])
  })

  it('диапазон: открытые порты таблицей с подсказкой', () => {
    const v = describeTool('scan', { host: 'web-01', from: 1, to: 1024, open: [22, 80, 999], scanned: 1024, ms: 2300 })!
    expect(v.verdict).toEqual({ tone: 'ok', text: 'Открыто 3 порта' })
    expect(v.sections?.[0]?.table?.rows).toEqual([
      ['22', 'SSH'],
      ['80', 'HTTP'],
      ['999', '']
    ])
    expect(v.rows).toContainEqual(['Время', '2,3 с'])
    expect(describeTool('scan', { host: 'x', from: 1, to: 10, open: [], scanned: 10 })!.verdict?.tone).toBe('info')
  })

  it('маршрут: неответившие узлы помечены', () => {
    const v = describeTool('trace', {
      host: '1.1.1.1',
      from_server: true,
      tool: 'tracepath',
      hops: [
        { n: 1, addr: '172.18.0.1', ms: 0.4 },
        { n: 2, addr: null, ms: null },
        { n: 3, addr: '1.1.1.1', ms: 12 }
      ]
    })!
    expect(v.verdict?.text).toBe('До 1.1.1.1: 3 узла')
    expect(v.sections?.[0]?.table?.rows[1]).toEqual(['2', '* не ответил', '—'])
    expect(v.note).toContain('Звёздочка')
    expect(describeTool('trace', { host: 'x', error: 'Маршрут построить не удалось' })!.verdict?.tone).toBe('bad')
  })

  it('HTTP: цепочка переадресаций, заголовки и тело', () => {
    const v = describeTool('http', {
      from_server: false,
      steps: [
        { url: 'http://example.com/', status: 301, reason: 'Moved Permanently', headers: [], ms: 20 },
        {
          url: 'https://example.com/',
          status: 200,
          reason: 'OK',
          headers: [{ name: 'Server', value: 'nginx' }],
          bodyBytes: 1256,
          bodyPreview: '<!doctype html>',
          truncated: false,
          ms: 41
        }
      ]
    })!
    expect(v.verdict).toEqual({ tone: 'ok', text: 'HTTP 200 OK' })
    expect(v.sections?.map((s) => s.title)).toEqual(['Переадресации', 'Заголовки ответа', 'Тело ответа'])
    expect(v.sections?.[0]?.table?.rows[0]).toEqual(['1', '301 Moved Permanently', 'http://example.com/'])

    // С сервера приходит один ответ без цепочки.
    const server = describeTool('http', {
      url: 'http://127.0.0.1/health',
      from_server: true,
      tool: 'curl',
      status: 502,
      reason: 'Bad Gateway',
      headers: []
    })!
    expect(server.verdict).toEqual({ tone: 'bad', text: 'HTTP 502 Bad Gateway' })
    expect(server.rows).toContainEqual(['Откуда', 'с сервера, через curl'])

    const failed = describeTool('http', { url: 'http://x', from_server: true, tool: 'curl', error: 'Запрос не состоялся' })!
    expect(failed.verdict).toEqual({ tone: 'bad', text: 'Запрос не состоялся' })
  })

  it('DNS: адреса и их тип, неразрешённое имя', () => {
    const v = describeTool('dns', { name: 'example.com', addresses: ['93.184.215.14', '2606:2800:21f::1'], latencyMs: 9 })!
    expect(v.verdict).toEqual({ tone: 'ok', text: 'example.com: 2 адреса' })
    expect(v.sections?.[0]?.table?.rows).toEqual([
      ['93.184.215.14', 'IPv4'],
      ['2606:2800:21f::1', 'IPv6']
    ])
    expect(describeTool('dns', { name: 'nope.invalid', from: 'server', tool: 'getent', addresses: [] })!.verdict).toEqual({
      tone: 'bad',
      text: 'Имя nope.invalid не разрешилось'
    })
  })

  it('TLS: сколько осталось до конца срока', () => {
    const cert = (afterDays: number, beforeDays = -60): unknown => ({
      host: 'nehade.xyz',
      port: 443,
      certificates: [
        {
          subject: 'CN=nehade.xyz',
          issuer: "C=US, O=Let's Encrypt, CN=E6",
          notBefore: 'x',
          notAfter: 'y',
          notBeforeTs: NOW / 1000 + beforeDays * DAY,
          notAfterTs: NOW / 1000 + afterDays * DAY + 3600,
          sha256: 'ab'.repeat(32),
          san: ['nehade.xyz', 'www.nehade.xyz']
        }
      ]
    })
    expect(describeTool('tls', cert(45), NOW)!.verdict).toEqual({ tone: 'ok', text: 'Действует ещё 45 дней' })
    expect(describeTool('tls', cert(3), NOW)!.verdict).toEqual({ tone: 'info', text: 'Истекает через 3 дня' })
    expect(describeTool('tls', cert(-2), NOW)!.verdict?.tone).toBe('bad')
    expect(describeTool('tls', cert(90, 2), NOW)!.verdict?.text).toBe('Сертификат ещё не вступил в силу')
    expect(describeTool('tls', cert(45), NOW)!.rows).toContainEqual(['Имена', 'nehade.xyz, www.nehade.xyz'])
  })

  it('LDAP: записи с атрибутами и пометка об обрезке', () => {
    const v = describeTool('ldap', {
      url: 'ldap://dc',
      base: 'dc=example,dc=com',
      filter: '(uid=*)',
      found: 120,
      entries: [{ dn: 'uid=ann,dc=example,dc=com', attrs: [{ name: 'mail', values: ['ann@example.com'] }] }],
      truncated: 'Показаны первые 50 из 120 - уточните условие поиска'
    })!
    expect(v.verdict?.text).toBe('Найдено 120 записей')
    expect(v.sections?.[0]).toEqual({ title: 'uid=ann,dc=example,dc=com', rows: [['mail', 'ann@example.com']] })
    expect(v.note).toContain('первые 50')
  })

  it('подсеть, хеш и JWT', () => {
    const net = describeTool('subnet', {
      network: '192.168.0.0',
      prefix: 24,
      netmask: '255.255.255.0',
      wildcard: '0.0.0.255',
      broadcast: '192.168.0.255',
      firstHost: '192.168.0.1',
      lastHost: '192.168.0.254',
      hostCount: 254
    })!
    expect(net.rows?.[0]).toEqual(['Сеть', '192.168.0.0/24'])
    expect(net.rows).toContainEqual(['Адресов для узлов', '254'])

    const h = describeTool('hash', { algo: 'sha256', hex: 'e3b0', base64: '47DE' })!
    expect(h.copyable).toBe(true)
    expect(h.rows?.[0]).toEqual(['SHA-256, hex', 'e3b0'])

    const expired = describeTool(
      'jwt',
      { header: { alg: 'HS256', typ: 'JWT' }, payload: { sub: '42', exp: NOW / 1000 - DAY }, signature: 'x' },
      NOW
    )!
    expect(expired.verdict?.tone).toBe('bad')
    expect(expired.rows).toContainEqual(['Субъект (sub)', '42'])
    expect(expired.note).toContain('не проверяется')
    expect(describeTool('jwt', { header: {}, payload: {}, signature: '' }, NOW)!.note).toBe('Подписи в токене нет.')
  })

  it('непонятный ответ не выдаётся за понятый', () => {
    expect(describeTool('port', 'просто строка')).toBeNull()
    expect(describeTool('diff', { same: true, lines: [] })).toBeNull()
  })
})
