/**
 * Оценка здоровья сервера: статус словами и причины.
 *
 * Не число и не цвет, а список того, что не так, - потому что вопрос к обзору всегда один:
 * «что чинить». Балл 0–100 на него не отвечает, а веса для балла пришлось бы выдумать.
 *
 * Правила вынесены сюда отдельно от панели и покрыты тестами: от них зависит, скажет ли
 * приложение «всё хорошо» там, где диск уже заканчивается.
 */

import type {
  HealthThresholds,
  MetricsPoint,
  ServerMetrics,
  Threshold,
  WorkspaceTool
} from '../shared/types'

/**
 * Пороги по умолчанию - те же, что панель показывала цветом и раньше: смена цвета и
 * оценка не должны расходиться. Диск строже процессора: заполнение растёт медленно, но
 * закончившееся место роняет всё сразу.
 */
export const DEFAULT_THRESHOLDS: HealthThresholds = {
  cpu: { warn: 60, bad: 85 },
  mem: { warn: 60, bad: 85 },
  disk: { warn: 75, bad: 90 },
  load: { warn: 0.7, bad: 1.0 }
}

/**
 * Сколько процессор должен держаться выше порога, чтобы это стало причиной.
 *
 * Всплески до ста процентов - нормальная работа сервера: сборка, архивация, запрос к базе.
 * Причина - это когда он не опускается пять минут подряд.
 */
export const CPU_SUSTAIN_MS = 5 * 60 * 1000

/** Разрыв между замерами, после которого данных за отрезок считаем нет. */
export const GAP_MS = 90 * 1000

export type Level = 'ok' | 'warn' | 'bad'

const KEYS = ['cpu', 'mem', 'disk', 'load'] as const

/** Порог приводится в порядок: «внимание» не может быть строже, чем «плохо». */
function tidy(t: Threshold, fallback: Threshold): Threshold {
  const warn = Number.isFinite(t.warn) ? t.warn : fallback.warn
  const bad = Number.isFinite(t.bad) ? t.bad : fallback.bad
  return { warn: Math.min(warn, bad), bad }
}

/**
 * Складывает пороги слоями: умолчания, общие из настроек, свои для сервера. Каждый
 * следующий слой перекрывает только то, что в нём задано.
 */
export function mergeThresholds(
  ...layers: (Partial<HealthThresholds> | undefined)[]
): HealthThresholds {
  const out: HealthThresholds = {
    cpu: { ...DEFAULT_THRESHOLDS.cpu },
    mem: { ...DEFAULT_THRESHOLDS.mem },
    disk: { ...DEFAULT_THRESHOLDS.disk },
    load: { ...DEFAULT_THRESHOLDS.load }
  }
  for (const layer of layers) {
    if (!layer) continue
    for (const key of KEYS) {
      const part = layer[key]
      if (part) out[key] = tidy({ ...out[key], ...part }, out[key])
    }
  }
  return out
}

export function levelOf(value: number, t: Threshold): Level {
  if (value >= t.bad) return 'bad'
  if (value >= t.warn) return 'warn'
  return 'ok'
}

export interface HealthReason {
  level: 'warn' | 'bad'
  text: string
  /** Куда смотреть, чтобы разобраться. */
  tool?: WorkspaceTool
}

export interface Health {
  level: Level | 'unknown'
  label: string
  reasons: HealthReason[]
}

/**
 * Сколько подряд, считая от последнего замера, значение не опускалось ниже порога.
 *
 * Разрыв в данных обрывает счёт: пока нас не было, процессор мог и отдыхать, и выдумывать
 * за него пять минут перегрузки нельзя.
 */
export function sustainedAbove(
  history: MetricsPoint[],
  pick: (p: MetricsPoint) => number,
  threshold: number
): { ms: number; min: number } {
  let start: MetricsPoint | undefined
  let min = Infinity
  for (let i = history.length - 1; i >= 0; i--) {
    const p = history[i]
    const value = pick(p)
    if (value < threshold) break
    if (start && start.t - p.t > GAP_MS) break
    start = p
    min = Math.min(min, value)
  }
  const last = history[history.length - 1]
  if (!start || !last) return { ms: 0, min: 0 }
  return { ms: last.t - start.t, min }
}

function servicesText(n: number): string {
  const mod100 = n % 100
  const mod10 = n % 10
  if (mod10 === 1 && mod100 !== 11) return `${n} служба упала`
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) return `${n} службы упали`
  return `${n} служб упало`
}

const LABEL: Record<Level, string> = { ok: 'Норма', warn: 'Внимание', bad: 'Плохо' }

export function evaluateHealth(
  m: ServerMetrics | null,
  history: MetricsPoint[],
  th: HealthThresholds
): Health {
  if (!m || !m.ok) return { level: 'unknown', label: 'Нет данных', reasons: [] }
  const reasons: HealthReason[] = []

  const cpu = sustainedAbove(history, (p) => p.cpu, th.cpu.warn)
  if (cpu.ms >= CPU_SUSTAIN_MS) {
    const bad = cpu.min >= th.cpu.bad
    reasons.push({
      level: bad ? 'bad' : 'warn',
      text: `процессор выше ${bad ? th.cpu.bad : th.cpu.warn}% уже ${Math.floor(cpu.ms / 60000)} мин`,
      tool: 'processes'
    })
  }

  const memPct = m.memTotalKb > 0 ? (m.memUsedKb / m.memTotalKb) * 100 : 0
  const memLevel = levelOf(memPct, th.mem)
  if (memLevel !== 'ok') {
    reasons.push({ level: memLevel, text: `память занята на ${Math.round(memPct)}%`, tool: 'processes' })
  }

  // Средняя загрузка за пять минут, а не за одну: минутная прыгает так же, как процессор.
  if (m.platform !== 'windows' && m.cores > 0) {
    const perCore = m.load[1] / m.cores
    const loadLevel = levelOf(perCore, th.load)
    if (loadLevel !== 'ok') {
      reasons.push({
        level: loadLevel,
        text: `занято ${m.load[1].toFixed(1)} из ${m.cores} ядер в среднем за 5 мин`,
        tool: 'processes'
      })
    }
  }

  const volumes = m.volumes?.length
    ? m.volumes
    : [{ mount: m.diskLabel ?? '/', usePct: m.diskPct }]
  for (const v of volumes) {
    const level = levelOf(v.usePct, th.disk)
    if (level !== 'ok') reasons.push({ level, text: `том ${v.mount} заполнен на ${v.usePct}%` })
  }

  if ((m.failedServices ?? 0) > 0) {
    reasons.push({ level: 'bad', text: servicesText(m.failedServices ?? 0), tool: 'services' })
  }

  reasons.sort((a, b) => (a.level === b.level ? 0 : a.level === 'bad' ? -1 : 1))
  const level: Level = reasons.some((r) => r.level === 'bad')
    ? 'bad'
    : reasons.length > 0
      ? 'warn'
      : 'ok'
  return { level, label: LABEL[level], reasons }
}
