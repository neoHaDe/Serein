/*
 * Ответ утилиты - словами, а не сырым JSON.
 *
 * Бэкенд отдаёт структуры, удобные машине: `{"ok": false, "error": "…"}`, массивы узлов,
 * секунды с 1970 года. Человеку нужен ответ на вопрос, который он задал («открыт ли порт»,
 * «до какого числа сертификат»), и подробности под ним. Здесь - только разбор, без React:
 * так его можно проверить тестами на тех же ответах, что приходят с сервера.
 */
import { timeText } from './actionLog'

export type Tone = 'ok' | 'bad' | 'info'

export interface ToolSection {
  title: string
  rows?: [string, string][]
  table?: { head: string[]; rows: string[][] }
  /** Готовый текст: тело ответа, JSON токена. */
  text?: string
}

export interface ToolView {
  /** Короткий ответ на заданный вопрос. */
  verdict?: { tone: Tone; text: string }
  /** Подпись → значение. */
  rows?: [string, string][]
  /** Значения строк стоит уметь скопировать: хеш, отпечаток. */
  copyable?: boolean
  sections?: ToolSection[]
  note?: string
}

type Obj = Record<string, unknown>

function obj(v: unknown): Obj | null {
  return typeof v === 'object' && v !== null && !Array.isArray(v) ? (v as Obj) : null
}
function str(v: unknown): string {
  return typeof v === 'string' ? v : typeof v === 'number' || typeof v === 'boolean' ? String(v) : ''
}
function num(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null
}
function list(v: unknown): unknown[] {
  return Array.isArray(v) ? v : []
}

/** Склонение по числу: 1 узел, 2 узла, 5 узлов. */
export function plural(n: number, one: string, few: string, many: string): string {
  const a = Math.abs(n) % 100
  const b = a % 10
  if (a > 10 && a < 20) return many
  if (b === 1) return one
  if (b >= 2 && b <= 4) return few
  return many
}

/** Длительность по-русски: дробь через запятую, ноль миллисекунд - «меньше 1 мс». */
export function msText(ms: number): string {
  if (ms === 0) return 'меньше 1 мс'
  if (ms >= 1000) return `${(ms / 1000).toFixed(1).replace('.', ',')} с`
  return `${String(ms).replace('.', ',')} мс`
}

/** Откуда смотрели: с этой машины или с сервера, и чем. */
function fromRow(v: Obj): [string, string] | null {
  const server = v.from === 'server' || v.from_server === true
  if (!server) return null
  const tool = str(v.tool)
  return ['Откуда', tool ? `с сервера, через ${tool}` : 'с сервера']
}

function withFrom(rows: [string, string][], v: Obj): [string, string][] {
  const f = fromRow(v)
  return f ? [...rows, f] : rows
}

/** Причина отказа TCP словами: системный текст бывает на любом языке и с кодом ошибки. */
function tcpReason(err: string): string {
  const e = err.toLowerCase()
  if (e.includes('refused') || e.includes('10061')) return 'в соединении отказано - на порту никто не слушает'
  if (e.includes('timed out') || e.includes('10060') || e.startsWith('таймаут')) {
    return `нет ответа (${err}) - порт закрыт фильтром или узел недоступен`
  }
  if (e.includes('unreachable') || e.includes('10065') || e.includes('10051')) return 'узел или сеть недоступны'
  return err
}

/** Известные порты - подсказка к открытому номеру, не утверждение. */
const KNOWN_PORTS: Record<number, string> = {
  21: 'FTP',
  22: 'SSH',
  23: 'telnet',
  25: 'SMTP',
  53: 'DNS',
  80: 'HTTP',
  110: 'POP3',
  143: 'IMAP',
  389: 'LDAP',
  443: 'HTTPS',
  445: 'SMB',
  465: 'SMTPS',
  587: 'SMTP (отправка)',
  636: 'LDAPS',
  993: 'IMAPS',
  995: 'POP3S',
  1433: 'MS SQL',
  1521: 'Oracle',
  2375: 'Docker (без TLS)',
  2376: 'Docker',
  3306: 'MySQL / MariaDB',
  3389: 'RDP',
  5432: 'PostgreSQL',
  5900: 'VNC',
  5901: 'VNC :1',
  6379: 'Redis',
  8080: 'HTTP (запасной)',
  8443: 'HTTPS (запасной)',
  9200: 'Elasticsearch',
  27017: 'MongoDB'
}

function port(v: Obj): ToolView {
  const where = `${str(v.host)}:${str(v.port)}`
  if (v.ok === true) {
    const ms = num(v.latencyMs)
    return {
      verdict: { tone: 'ok', text: `Порт ${where} открыт` },
      rows: withFrom(ms != null ? [['Отклик', msText(ms)]] : [], v)
    }
  }
  return {
    verdict: { tone: 'bad', text: `Порт ${where} закрыт или недоступен` },
    rows: withFrom(str(v.error) ? [['Причина', tcpReason(str(v.error))]] : [], v)
  }
}

function scan(v: Obj): ToolView {
  if (str(v.error)) return { verdict: { tone: 'bad', text: str(v.error) } }
  const open = list(v.open).map(num).filter((p): p is number => p != null)
  const scanned = num(v.scanned) ?? 0
  const range = `${str(v.from)}–${str(v.to)}`
  const rows: [string, string][] = [
    ['Узел', str(v.host)],
    ['Диапазон', `${range}, ${scanned} ${plural(scanned, 'порт', 'порта', 'портов')}`]
  ]
  const ms = num(v.ms)
  if (ms != null) rows.push(['Время', msText(ms)])
  if (!open.length) {
    return {
      verdict: { tone: 'info', text: `Открытых портов в диапазоне ${range} нет` },
      rows: withFrom(rows, v)
    }
  }
  return {
    verdict: { tone: 'ok', text: `Открыто ${open.length} ${plural(open.length, 'порт', 'порта', 'портов')}` },
    rows: withFrom(rows, v),
    sections: [
      {
        title: 'Открытые порты',
        table: { head: ['Порт', 'Обычно это'], rows: open.map((p) => [String(p), KNOWN_PORTS[p] ?? '']) }
      }
    ]
  }
}

function trace(v: Obj): ToolView {
  if (str(v.error)) return { verdict: { tone: 'bad', text: str(v.error) } }
  const hops = list(v.hops).map(obj).filter((h): h is Obj => h != null)
  const n = hops.length
  return {
    verdict: { tone: 'info', text: `До ${str(v.host)}: ${n} ${plural(n, 'узел', 'узла', 'узлов')}` },
    rows: withFrom([], v),
    sections: [
      {
        title: 'Маршрут',
        table: {
          head: ['№', 'Узел', 'Отклик'],
          rows: hops.map((h) => {
            const ms = num(h.ms)
            return [str(h.n), str(h.addr) || '* не ответил', ms != null ? msText(ms) : '—']
          })
        }
      }
    ],
    note: n && hops.some((h) => !str(h.addr))
      ? 'Звёздочка - узел не ответил на проверку. Это обычно фильтр, а не обрыв: дальше маршрут может идти.'
      : undefined
  }
}

function statusTone(code: number): Tone {
  if (code >= 200 && code < 300) return 'ok'
  if (code >= 300 && code < 400) return 'info'
  return 'bad'
}

function http(v: Obj): ToolView {
  // С этой машины приходит цепочка шагов, с сервера - один итоговый ответ.
  const steps = v.steps !== undefined ? list(v.steps).map(obj).filter((s): s is Obj => s != null) : [v]
  const last = steps[steps.length - 1]
  const status = last ? num(last.status) : null
  if (!last || status == null) {
    return { verdict: { tone: 'bad', text: str(v.error) || 'Ответа нет' }, rows: withFrom([], v) }
  }
  const rows: [string, string][] = [['Адрес', str(last.url)]]
  const ms = num(last.ms)
  if (ms != null) rows.push(['Время', msText(ms)])
  const size = num(last.bodyBytes)
  if (size != null) rows.push(['Размер тела', `${size} Б`])
  const sections: ToolSection[] = []
  if (steps.length > 1) {
    sections.push({
      title: 'Переадресации',
      table: {
        head: ['Шаг', 'Ответ', 'Адрес'],
        rows: steps.map((s, i) => [String(i + 1), `${str(s.status)} ${str(s.reason)}`.trim(), str(s.url)])
      }
    })
  }
  const headers = list(last.headers).map(obj).filter((h): h is Obj => h != null)
  if (headers.length) {
    sections.push({
      title: 'Заголовки ответа',
      table: { head: ['Заголовок', 'Значение'], rows: headers.map((h) => [str(h.name), str(h.value)]) }
    })
  }
  const body = str(last.bodyPreview)
  if (body) {
    sections.push({ title: last.truncated === true ? 'Начало тела ответа' : 'Тело ответа', text: body })
  }
  return {
    verdict: { tone: statusTone(status), text: `HTTP ${status} ${str(last.reason)}`.trim() },
    rows: withFrom(rows, v),
    sections,
    note: str(v.error) || undefined
  }
}

function dns(v: Obj): ToolView {
  if (str(v.error)) return { verdict: { tone: 'bad', text: str(v.error) }, rows: withFrom([], v) }
  const addrs = list(v.addresses).map(str).filter(Boolean)
  const rows: [string, string][] = []
  const ms = num(v.latencyMs)
  if (ms != null) rows.push(['Время', msText(ms)])
  if (!addrs.length) {
    return { verdict: { tone: 'bad', text: `Имя ${str(v.name)} не разрешилось` }, rows: withFrom(rows, v) }
  }
  return {
    verdict: {
      tone: 'ok',
      text: `${str(v.name)}: ${addrs.length} ${plural(addrs.length, 'адрес', 'адреса', 'адресов')}`
    },
    rows: withFrom(rows, v),
    sections: [
      {
        title: 'Адреса',
        table: { head: ['Адрес', 'Тип'], rows: addrs.map((a) => [a, a.includes(':') ? 'IPv6' : 'IPv4']) }
      }
    ]
  }
}

/** Дата из секунд с 1970 года - в часовом поясе машины. */
function dateFromTs(ts: number | null): string {
  return ts == null ? '' : timeText(new Date(ts * 1000).toISOString())
}

const DAY = 86_400_000

function tls(v: Obj, now: number): ToolView {
  const cert = obj(list(v.certificates)[0])
  if (!cert) return { verdict: { tone: 'bad', text: 'Сертификата в ответе нет' } }
  const until = num(cert.notAfterTs)
  const since = num(cert.notBeforeTs)
  const rows: [string, string][] = [
    ['Кому выдан', str(cert.subject)],
    ['Кем выдан', str(cert.issuer)],
    ['Действует с', dateFromTs(since) || str(cert.notBefore)],
    ['Действует до', dateFromTs(until) || str(cert.notAfter)]
  ]
  const san = list(cert.san).map(str).filter(Boolean)
  if (san.length) rows.push(['Имена', san.join(', ')])
  rows.push(['SHA-256', str(cert.sha256)])
  let verdict: ToolView['verdict']
  if (until != null) {
    const days = Math.floor((until * 1000 - now) / DAY)
    if (days < 0) verdict = { tone: 'bad', text: `Сертификат истёк ${-days} ${plural(-days, 'день', 'дня', 'дней')} назад` }
    else if (since != null && since * 1000 > now) verdict = { tone: 'bad', text: 'Сертификат ещё не вступил в силу' }
    else if (days < 30) verdict = { tone: 'info', text: `Истекает через ${days} ${plural(days, 'день', 'дня', 'дней')}` }
    else verdict = { tone: 'ok', text: `Действует ещё ${days} ${plural(days, 'день', 'дня', 'дней')}` }
  }
  return {
    verdict,
    rows,
    note: 'Цепочка доверия и совпадение имени здесь не проверяются - показано то, что прислал сервер.'
  }
}

function ldap(v: Obj): ToolView {
  const found = num(v.found) ?? 0
  const entries = list(v.entries).map(obj).filter((e): e is Obj => e != null)
  const rows: [string, string][] = [
    ['Каталог', str(v.url)],
    ['Откуда искали', str(v.base) || '(корень)'],
    ['Условие', str(v.filter)]
  ]
  const ms = num(v.ms)
  if (ms != null) rows.push(['Время', msText(ms)])
  return {
    verdict: found
      ? { tone: 'ok', text: `Найдено ${found} ${plural(found, 'запись', 'записи', 'записей')}` }
      : { tone: 'info', text: 'Ничего не найдено' },
    rows,
    sections: entries.map((e) => ({
      title: str(e.dn) || '(без имени)',
      rows: list(e.attrs)
        .map(obj)
        .filter((a): a is Obj => a != null)
        .map((a): [string, string] => [str(a.name), list(a.values).map(str).join(', ')])
    })),
    note: str(v.truncated) || undefined
  }
}

function subnet(v: Obj): ToolView {
  const hosts = num(v.hostCount) ?? 0
  return {
    rows: [
      ['Сеть', `${str(v.network)}/${str(v.prefix)}`],
      ['Маска', str(v.netmask)],
      ['Обратная маска', str(v.wildcard)],
      ['Широковещательный', str(v.broadcast)],
      ['Первый адрес узла', str(v.firstHost)],
      ['Последний адрес узла', str(v.lastHost)],
      ['Адресов для узлов', String(hosts)]
    ],
    note: hosts === 0 ? 'В сетях /31 и /32 отдельных адресов сети и широковещания нет.' : undefined
  }
}

const HASH_NAMES: Record<string, string> = { md5: 'MD5', sha1: 'SHA-1', sha256: 'SHA-256', sha512: 'SHA-512' }

function hash(v: Obj): ToolView {
  const algo = str(v.algo)
  return {
    rows: [
      [`${HASH_NAMES[algo] ?? algo}, hex`, str(v.hex)],
      ['Base64', str(v.base64)]
    ],
    copyable: true
  }
}

/** Время из поля токена (`exp`, `iat`, `nbf` - секунды с 1970 года). */
function claimTime(p: Obj, key: string): number | null {
  return num(p[key])
}

function jwt(v: Obj, now: number): ToolView {
  const header = obj(v.header) ?? {}
  const payload = obj(v.payload) ?? {}
  const rows: [string, string][] = []
  if (str(header.alg)) rows.push(['Алгоритм подписи', str(header.alg)])
  for (const [key, label] of [
    ['iss', 'Кем выдан (iss)'],
    ['sub', 'Субъект (sub)'],
    ['aud', 'Для кого (aud)']
  ] as const) {
    const val = payload[key]
    const text = Array.isArray(val) ? val.map(str).join(', ') : str(val)
    if (text) rows.push([label, text])
  }
  for (const [key, label] of [
    ['iat', 'Выдан (iat)'],
    ['nbf', 'Действует с (nbf)'],
    ['exp', 'Истекает (exp)']
  ] as const) {
    const t = claimTime(payload, key)
    if (t != null) rows.push([label, dateFromTs(t)])
  }
  const exp = claimTime(payload, 'exp')
  let verdict: ToolView['verdict']
  if (exp != null) {
    verdict =
      exp * 1000 < now
        ? { tone: 'bad', text: `Срок токена истёк ${dateFromTs(exp)}` }
        : { tone: 'ok', text: `Токен действует до ${dateFromTs(exp)}` }
  }
  return {
    verdict,
    rows,
    sections: [
      { title: 'Заголовок', text: JSON.stringify(header, null, 2) },
      { title: 'Содержимое', text: JSON.stringify(payload, null, 2) }
    ],
    note: str(v.signature) ? 'Подпись есть, но не проверяется: для этого нужен ключ.' : 'Подписи в токене нет.'
  }
}

/**
 * Ответ утилиты в виде для человека. `null` - разобрать не удалось, и тогда окно покажет
 * ответ как есть: лучше сырой текст, чем пустое место.
 */
export function describeTool(tab: string, value: unknown, now: number = Date.now()): ToolView | null {
  const v = obj(value)
  if (!v) return null
  switch (tab) {
    case 'port':
      return port(v)
    case 'scan':
      return scan(v)
    case 'trace':
      return trace(v)
    case 'http':
      return http(v)
    case 'dns':
      return dns(v)
    case 'tls':
      return tls(v, now)
    case 'ldap':
      return ldap(v)
    case 'subnet':
      return subnet(v)
    case 'hash':
      return hash(v)
    case 'jwt':
      return jwt(v, now)
    default:
      return null
  }
}
