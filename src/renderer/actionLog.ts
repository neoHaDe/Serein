/**
 * Журнал действий в окне: как назвать запись словами и что показать в строке.
 *
 * Сами записи пишет бэкенд (`actionlog.rs`); здесь только чтение. Вынесено и покрыто
 * тестами: по этой таблице служба безопасности разбирает, что делали на серверах, и
 * «Подробности» не должны терять главное - команду, путь, запрос.
 */

import { matchesQuery } from './tableSort'

export interface ActionLogEntry {
  seq: number
  t: string
  actor?: { user?: string; machine?: string }
  server?: { id: string; name?: string | null; host?: string | null; port?: number | null; user?: string | null } | null
  session?: string | null
  action: string
  detail?: Record<string, unknown> | null
  ok: boolean
  error?: string | null
}

export interface ActionLogVerify {
  ok: boolean
  count: number
  files?: number
  file?: string
  line?: number
  reason?: string
}

export interface ActionLogStatus {
  enabled: boolean
  dir: string
  syslog: { host: string; port: number; tcp: boolean } | null
  syslogSent: number
  syslogFailed: number
  /** Сколько записей не легло на диск и почему не легла последняя. */
  writeFailed: number
  lastWriteError: string | null
  /** Сколько раз замок достался отравленным: где-то паника прервала правку состояния. */
  locksPoisoned: number
}

const LABELS: Record<string, string> = {
  'app.start': 'Запуск Serein',
  'journal.enabled': 'Журнал включён',
  'journal.disabled': 'Журнал выключен',
  'ssh.connect': 'Подключение',
  'ssh.disconnect': 'Отключение',
  'terminal.line': 'Строка в терминале',
  'file.mkdir': 'Создан каталог',
  'file.remove': 'Удаление',
  'file.rename': 'Переименование',
  'file.chmod': 'Права доступа',
  'file.save': 'Сохранение файла',
  'file.upload': 'Заливка',
  'file.download': 'Скачивание',
  'file.edit': 'Внешний редактор',
  'file.edit.upload': 'Правка залита',
  'docker.action': 'Контейнер',
  'docker.compose': 'Compose',
  'process.kill': 'Завершение процесса',
  'service.action': 'Служба',
  'db.open': 'Подключение к базе',
  'db.query': 'Запрос к базе',
  'fleet.exec': 'Fleet',
  'task.run': 'Задача',
  'tunnel.open': 'Туннель',
  'rdp.open': 'RDP',
  'vnc.open': 'VNC'
}

export function actionLabel(action: string): string {
  return LABELS[action] ?? action
}

/** Сервер записи для человека: имя и адрес. */
export function serverText(e: ActionLogEntry): string {
  const s = e.server
  if (!s) return ''
  const addr = s.host ? `${s.user ? s.user + '@' : ''}${s.host}${s.port && s.port !== 22 ? ':' + s.port : ''}` : ''
  if (s.name && addr) return `${s.name} (${addr})`
  return s.name || addr || s.id
}

function scalar(v: unknown): string | null {
  if (typeof v === 'string') return v
  if (typeof v === 'number' || typeof v === 'boolean') return String(v)
  return null
}

/** Подробности одной строкой. Главное - первым: команда, запрос, путь. */
export function detailText(e: ActionLogEntry): string {
  const d = e.detail ?? {}
  if (e.action === 'terminal.line') {
    if (typeof d.hidden === 'string') return d.hidden
    const line = typeof d.line === 'string' ? d.line : ''
    if (d.edited) {
      return line
        ? `${line} (строку дописал или заменил сервер - здесь набранное с клавиатуры)`
        : '(команда из истории или автодополнения - текст неизвестен)'
    }
    return line
  }
  const first = ['command', 'query', 'path', 'remotePath', 'from', 'task', 'container', 'name', 'pid']
  const parts: string[] = []
  for (const k of first) {
    const v = scalar(d[k])
    if (v !== null && v !== '') parts.push(k === 'from' && scalar(d.to) ? `${v} → ${scalar(d.to)}` : v)
  }
  for (const [k, v] of Object.entries(d)) {
    if (first.includes(k) || k === 'to') continue
    const s = scalar(v)
    if (s !== null && s !== '') parts.push(`${k}: ${s}`)
    else if (Array.isArray(v)) parts.push(`${k}: ${v.length}`)
  }
  return parts.join(' · ')
}

/** Время записи в часовом поясе машины. */
export function timeText(t: string): string {
  const d = new Date(t)
  if (Number.isNaN(d.getTime())) return t
  const pad = (n: number): string => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

export function matchesEntry(e: ActionLogEntry, query: string): boolean {
  return matchesQuery(
    [
      e.seq,
      timeText(e.t),
      e.actor?.user,
      e.actor?.machine,
      serverText(e),
      actionLabel(e.action),
      e.action,
      detailText(e),
      e.error ?? undefined
    ],
    query
  )
}

/** Итог проверки целостности словами. */
export function verifyText(v: ActionLogVerify): string {
  if (v.ok) return `Цепочка цела: ${v.count} записей в ${v.files ?? 0} файлах.`
  return `Разрыв после записи ${v.count}: ${v.file}, строка ${v.line} - ${v.reason}.`
}
