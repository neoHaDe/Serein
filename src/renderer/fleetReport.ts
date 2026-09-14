/**
 * Правила вокруг прогона одной команды на нескольких серверах: сводка, повтор, отчёт.
 *
 * Вынесено из окна и покрыто тестами, потому что по этим числам решают, что делать дальше:
 * «успешно 18, код 1: 2» и «успешно 20» - это разные решения, и перепутать их нельзя.
 */
import type { MultiExecResult } from '../shared/types'

/** Готовые варианты числа одновременных хостов. */
export const CONCURRENCY_PRESETS = [1, 4, 8, 16, 32]
export const DEFAULT_CONCURRENCY = 4
/** Совпадает с пределом бэкенда: больше - десятки рукопожатий разом с одной машины. */
export const MAX_CONCURRENCY = 64
export const DEFAULT_TIMEOUT_SEC = 120
export const MAX_TIMEOUT_SEC = 3600

export function clampConcurrency(n: number): number {
  if (!Number.isFinite(n)) return DEFAULT_CONCURRENCY
  return Math.min(MAX_CONCURRENCY, Math.max(1, Math.round(n)))
}

export function clampTimeout(n: number): number {
  if (!Number.isFinite(n)) return DEFAULT_TIMEOUT_SEC
  return Math.min(MAX_TIMEOUT_SEC, Math.max(1, Math.round(n)))
}

/** Состояние хоста словами - одинаково в окне и в отчёте. */
export function stateLabel(r: MultiExecResult): string {
  if (r.state === 'skipped') return 'пропущен'
  if (r.state === 'failed') return 'не дошли'
  return r.code === 0 ? 'готово' : `код ${r.code}`
}

/**
 * Упал ли хост: не дошли до него или команда вернула не ноль.
 *
 * Пропущенные упавшими не считаются: их пропустили из-за неподтверждённого ключа, и повтор
 * пропустит их снова - кнопка «повторить» обещала бы то, чего не сделает.
 */
export function isFailed(r: MultiExecResult): boolean {
  return r.state === 'failed' || (r.state === 'done' && r.code !== 0)
}

export function retryIds(results: MultiExecResult[]): string[] {
  return results.filter(isFailed).map((r) => r.serverId)
}

/**
 * Новые результаты поверх прежних. Повторённый хост заменяет свою строку, а не дописывается
 * второй: иначе в списке висели бы и старая ошибка, и новый успех, и итог врал бы.
 */
export function mergeResults(prev: MultiExecResult[], next: MultiExecResult[]): MultiExecResult[] {
  const fresh = new Map(next.map((r) => [r.serverId, r]))
  const known = new Set(prev.map((r) => r.serverId))
  return [...prev.map((r) => fresh.get(r.serverId) ?? r), ...next.filter((r) => !known.has(r.serverId))]
}

export interface FleetSummary {
  ok: number
  /** Ненулевые коды возврата и сколько хостов с каждым, по возрастанию кода. */
  codes: [number, number][]
  failed: number
  skipped: number
}

export function summarizeFleet(results: MultiExecResult[]): FleetSummary {
  let ok = 0
  let failed = 0
  let skipped = 0
  const codes = new Map<number, number>()
  for (const r of results) {
    if (r.state === 'skipped') skipped++
    else if (r.state === 'failed') failed++
    else if (r.code === 0) ok++
    else codes.set(r.code ?? -1, (codes.get(r.code ?? -1) ?? 0) + 1)
  }
  return { ok, codes: [...codes.entries()].sort((a, b) => a[0] - b[0]), failed, skipped }
}

export function summaryText(s: FleetSummary): string {
  const parts: string[] = []
  if (s.ok) parts.push(`успешно ${s.ok}`)
  for (const [code, n] of s.codes) parts.push(`код ${code}: ${n}`)
  if (s.failed) parts.push(`не дошли ${s.failed}`)
  if (s.skipped) parts.push(`пропущено ${s.skipped}`)
  return parts.join(' · ') || 'нет результатов'
}

/** Упавшие - первыми: отчёт открывают ради них. */
function rank(r: MultiExecResult): number {
  if (isFailed(r)) return 0
  if (r.state === 'skipped') return 1
  return 2
}

/** Отчёт прогона текстом: сводка сверху, дальше хосты - сначала упавшие, потом остальные. */
export function fleetReport(command: string, results: MultiExecResult[], when: Date): string {
  const lines = [
    `Команда: ${command}`,
    `Время: ${when.toLocaleString('ru-RU')}`,
    `Хостов: ${results.length}`,
    `Итог: ${summaryText(summarizeFleet(results))}`,
    ''
  ]
  const ordered = results
    .map((r, i) => ({ r, i }))
    .sort((a, b) => rank(a.r) - rank(b.r) || a.i - b.i)
    .map(({ r }) => r)
  for (const r of ordered) {
    lines.push(`=== ${r.name} - ${stateLabel(r)}${r.ms !== undefined ? ` · ${r.ms} мс` : ''}`)
    const body = r.error ?? [r.stdout, r.stderr].filter(Boolean).join('\n')
    if (body) lines.push(body.trimEnd())
    lines.push('')
  }
  return lines.join('\n')
}
