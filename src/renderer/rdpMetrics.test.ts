import { describe, expect, it } from 'vitest'
import { RdpUiMetrics } from './rdpMetrics'

describe('метрики RDP UI', () => {
  it('считает скорость и перцентили за интервал', () => {
    const metrics = new RdpUiMetrics(1_000)
    metrics.noteFrame(1024 * 1024, 1)
    metrics.noteFrame(1024 * 1024, 9)
    metrics.noteRaw(2_000_000, 4)
    metrics.noteRaw(1_000_000, 12)
    metrics.notePresent(2)
    metrics.notePresent(8)

    const result = metrics.take(3_000)
    expect(result.frames).toBe(2)
    expect(result.fps).toBe(1)
    expect(result.ipcMiBps).toBe(1)
    expect(result.rawMegapixels).toBe(3)
    expect(result.decodeP50Ms).toBe(1)
    expect(result.decodeP95Ms).toBe(9)
    expect(result.drawP95Ms).toBe(12)
    expect(result.presentP95Ms).toBe(8)
  })

  it('сбрасывает интервал после отчёта', () => {
    const metrics = new RdpUiMetrics(0)
    metrics.noteFrame(500, 2)
    void metrics.take(5_000)
    const empty = metrics.take(10_000)
    expect(empty.frames).toBe(0)
    expect(empty.ipcMiBps).toBe(0)
  })
})
