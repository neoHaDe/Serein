export interface RdpUiMetricSnapshot {
  intervalMs: number
  frames: number
  fps: number
  ipcMiBps: number
  rawFrames: number
  rawMegapixels: number
  presents: number
  decodeP50Ms: number
  decodeP95Ms: number
  drawP50Ms: number
  drawP95Ms: number
  presentP50Ms: number
  presentP95Ms: number
}

const REPORT_INTERVAL_MS = 5_000
const MAX_SAMPLES = 2_000

function addBounded(samples: number[], value: number): void {
  if (!Number.isFinite(value) || value < 0) return
  if (samples.length < MAX_SAMPLES) samples.push(value)
}

function percentile(samples: number[], part: number): number {
  if (!samples.length) return 0
  const sorted = [...samples].sort((a, b) => a - b)
  const index = Math.min(sorted.length - 1, Math.ceil(sorted.length * part) - 1)
  return Number(sorted[Math.max(0, index)].toFixed(3))
}

/** Небольшой накопитель для диагностического журнала RDP, без React-state на каждый кадр. */
export class RdpUiMetrics {
  private startedAt: number
  private frames = 0
  private bytes = 0
  private rawFrames = 0
  private rawPixels = 0
  private presents = 0
  private decodeMs: number[] = []
  private drawMs: number[] = []
  private presentMs: number[] = []

  constructor(now: number) {
    this.startedAt = now
  }

  noteFrame(bytes: number, decodeMs: number): void {
    this.frames += 1
    this.bytes += Math.max(0, bytes)
    addBounded(this.decodeMs, decodeMs)
  }

  noteRaw(pixels: number, drawMs: number): void {
    this.rawFrames += 1
    this.rawPixels += Math.max(0, pixels)
    addBounded(this.drawMs, drawMs)
  }

  notePresent(durationMs: number): void {
    this.presents += 1
    addBounded(this.presentMs, durationMs)
  }

  due(now: number): boolean {
    return now - this.startedAt >= REPORT_INTERVAL_MS
  }

  take(now: number): RdpUiMetricSnapshot {
    const intervalMs = Math.max(1, now - this.startedAt)
    const seconds = intervalMs / 1_000
    const result: RdpUiMetricSnapshot = {
      intervalMs: Math.round(intervalMs),
      frames: this.frames,
      fps: Number((this.frames / seconds).toFixed(2)),
      ipcMiBps: Number((this.bytes / 1024 / 1024 / seconds).toFixed(3)),
      rawFrames: this.rawFrames,
      rawMegapixels: Number((this.rawPixels / 1_000_000).toFixed(3)),
      presents: this.presents,
      decodeP50Ms: percentile(this.decodeMs, 0.5),
      decodeP95Ms: percentile(this.decodeMs, 0.95),
      drawP50Ms: percentile(this.drawMs, 0.5),
      drawP95Ms: percentile(this.drawMs, 0.95),
      presentP50Ms: percentile(this.presentMs, 0.5),
      presentP95Ms: percentile(this.presentMs, 0.95)
    }
    this.startedAt = now
    this.frames = 0
    this.bytes = 0
    this.rawFrames = 0
    this.rawPixels = 0
    this.presents = 0
    this.decodeMs = []
    this.drawMs = []
    this.presentMs = []
    return result
  }
}
