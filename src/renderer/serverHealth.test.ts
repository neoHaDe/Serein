import { describe, expect, it } from 'vitest'
import type { MetricsPoint, ServerMetrics } from '../shared/types'
import {
  CPU_SUSTAIN_MS,
  DEFAULT_THRESHOLDS,
  evaluateHealth,
  mergeThresholds,
  sustainedAbove
} from './serverHealth'

const metrics = (over: Partial<ServerMetrics> = {}): ServerMetrics => ({
  ok: true,
  cores: 4,
  cpuPct: 10,
  load: [0.4, 0.4, 0.4],
  memTotalKb: 1000,
  memUsedKb: 200,
  diskPct: 30,
  uptimeSec: 100,
  ...over
})

/** Замеры раз в 3 с с заданным процессором на протяжении `ms`. */
const cpuFor = (ms: number, cpu: number, end = 10_000_000): MetricsPoint[] => {
  const out: MetricsPoint[] = []
  for (let t = end - ms; t <= end; t += 3000) out.push({ t, cpu, mem: 20, disk: 30, cores: 4 })
  return out
}

describe('пороги', () => {
  it('без настроек действуют умолчания', () => {
    expect(mergeThresholds()).toEqual(DEFAULT_THRESHOLDS)
  })

  it('свои пороги сервера перекрывают общие, а общие - умолчания', () => {
    const th = mergeThresholds({ mem: { warn: 70, bad: 90 } }, { mem: { warn: 80, bad: 95 } })
    expect(th.mem).toEqual({ warn: 80, bad: 95 })
    expect(th.cpu).toEqual(DEFAULT_THRESHOLDS.cpu)
  })

  it('«внимание» не бывает строже «плохо»', () => {
    const th = mergeThresholds({ disk: { warn: 95, bad: 80 } })
    expect(th.disk.warn).toBeLessThanOrEqual(th.disk.bad)
  })
})

describe('здоровье', () => {
  it('без данных - так и говорим, а не «норма»', () => {
    expect(evaluateHealth(null, [], DEFAULT_THRESHOLDS).level).toBe('unknown')
    expect(evaluateHealth(metrics({ ok: false }), [], DEFAULT_THRESHOLDS).label).toBe('Нет данных')
  })

  it('спокойный сервер - норма без причин', () => {
    const h = evaluateHealth(metrics(), [], DEFAULT_THRESHOLDS)
    expect(h).toEqual({ level: 'ok', label: 'Норма', reasons: [] })
  })

  it('короткий всплеск процессора причиной не считается', () => {
    const h = evaluateHealth(metrics({ cpuPct: 99 }), cpuFor(2 * 60_000, 99), DEFAULT_THRESHOLDS)
    expect(h.reasons).toEqual([])
  })

  it('процессор выше порога дольше пяти минут - причина', () => {
    const h = evaluateHealth(metrics({ cpuPct: 99 }), cpuFor(CPU_SUSTAIN_MS + 60_000, 99), DEFAULT_THRESHOLDS)
    expect(h.level).toBe('bad')
    expect(h.reasons[0].text).toContain('процессор выше 85%')
    expect(h.reasons[0].tool).toBe('processes')
  })

  it('разрыв в замерах обрывает счёт перегрузки', () => {
    const раньше = cpuFor(4 * 60_000, 99, 5_000_000)
    const потом = cpuFor(4 * 60_000, 99, 5_000_000 + 4 * 60_000 + 10 * 60_000)
    expect(sustainedAbove([...раньше, ...потом], (p) => p.cpu, 60).ms).toBeLessThan(CPU_SUSTAIN_MS)
  })

  it('заполненный том называется по имени', () => {
    const h = evaluateHealth(
      metrics({ volumes: [{ mount: '/mnt/data', sizeKb: 100, usedKb: 93, usePct: 93 }] }),
      [],
      DEFAULT_THRESHOLDS
    )
    expect(h.level).toBe('bad')
    expect(h.reasons[0].text).toBe('том /mnt/data заполнен на 93%')
  })

  it('упавшие службы - всегда «плохо» и ведут к списку служб', () => {
    const h = evaluateHealth(metrics({ failedServices: 2 }), [], DEFAULT_THRESHOLDS)
    expect(h.level).toBe('bad')
    expect(h.reasons[0]).toEqual({ level: 'bad', text: '2 службы упали', tool: 'services' })
  })

  it('у Windows средней загрузки нет - и причины по ней тоже', () => {
    const h = evaluateHealth(metrics({ platform: 'windows', load: [99, 99, 99] }), [], DEFAULT_THRESHOLDS)
    expect(h.reasons.some((r) => r.text.includes('ядер'))).toBe(false)
  })

  it('свой порог сервера меняет оценку', () => {
    const m = metrics({ memUsedKb: 880 })
    expect(evaluateHealth(m, [], DEFAULT_THRESHOLDS).level).toBe('bad')
    expect(evaluateHealth(m, [], mergeThresholds({ mem: { warn: 90, bad: 97 } })).level).toBe('ok')
  })
})
