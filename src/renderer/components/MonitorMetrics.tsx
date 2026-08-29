import { useEffect, useRef, useState } from 'react'
import type { ServerMetrics } from '../../shared/types'
import { Icon } from './Icon'
import { errText } from '../errText'

function fmtKb(kb: number): string {
  const mb = kb / 1024
  if (mb < 1024) return `${mb.toFixed(0)} МБ`
  return `${(mb / 1024).toFixed(1)} ГБ`
}

function fmtUptime(sec: number): string {
  const d = Math.floor(sec / 86400)
  const h = Math.floor((sec % 86400) / 3600)
  const m = Math.floor((sec % 3600) / 60)
  if (d > 0) return `${d}д ${h}ч`
  if (h > 0) return `${h}ч ${m}м`
  return `${m}м`
}

function tone(pct: number): 'ok' | 'warn' | 'bad' {
  if (pct < 60) return 'ok'
  if (pct < 85) return 'warn'
  return 'bad'
}

function loadTone(load: number, cores: number): 'ok' | 'warn' | 'bad' {
  const ratio = cores > 0 ? load / cores : load
  if (ratio < 0.7) return 'ok'
  if (ratio < 1.0) return 'warn'
  return 'bad'
}

function loadLabel(load: number, cores: number): string {
  const ratio = cores > 0 ? load / cores : load
  if (ratio < 0.5) return 'легко'
  if (ratio < 0.85) return 'умеренно'
  if (ratio < 1.0) return 'напряжённо'
  return 'перегруз'
}

function Gauge({ pct, label, detail }: { pct: number; label: string; detail: string }): JSX.Element {
  const clamped = Math.min(100, Math.max(0, pct))
  const t = tone(clamped)
  return (
    <div className={'srv-gauge-card tone-' + t}>
      <div
        className="srv-gauge-ring"
        style={{ ['--pct' as string]: String(clamped) }}
        aria-hidden
      >
        <span className="srv-gauge-val">{Math.round(clamped)}%</span>
      </div>
      <div className="srv-gauge-meta">
        <span className="srv-gauge-label">{label}</span>
        <span className="srv-gauge-detail">{detail}</span>
      </div>
    </div>
  )
}

function LoadRow({
  label,
  load,
  cores
}: {
  label: string
  load: number
  cores: number
}): JSX.Element {
  const cap = cores > 0 ? (load / cores) * 100 : 0
  const t = loadTone(load, cores)
  return (
    <div className="srv-load-row">
      <span className="srv-load-label">{label}</span>
      <div className="srv-load-track" title={`${load.toFixed(2)} на ${cores} яд.`}>
        <div className={'srv-load-fill tone-' + t} style={{ width: `${Math.min(100, cap)}%` }} />
      </div>
      <span className="srv-load-num mono">{load.toFixed(2)}</span>
      <span className={'srv-load-tag tone-' + t}>{loadLabel(load, cores)}</span>
    </div>
  )
}

function MetricsDashboard({ m }: { m: ServerMetrics }): JSX.Element {
  const memPct = m.memTotalKb > 0 ? (m.memUsedKb / m.memTotalKb) * 100 : 0
  const memFreeKb = m.memTotalKb > m.memUsedKb ? m.memTotalKb - m.memUsedKb : 0
  const load1pct = m.cores > 0 ? (m.load[0] / m.cores) * 100 : 0

  return (
    <div className="srv-dash">
      <div className="srv-dash-gauges">
        <Gauge label="CPU" pct={m.cpuPct} detail={`${m.cpuPct}% · ${m.cores} яд.`} />
        <Gauge
          label="RAM"
          pct={memPct}
          detail={`${fmtKb(m.memUsedKb)} / ${fmtKb(m.memTotalKb)}`}
        />
        <Gauge label="Диск /" pct={m.diskPct} detail={`занято ${m.diskPct}%`} />
        <div className="srv-dash-summary">
          <div className="srv-dash-summary-row">
            <span className="srv-dash-summary-k">Свободно RAM</span>
            <span className="srv-dash-summary-v mono">{fmtKb(memFreeKb)}</span>
          </div>
          <div className="srv-dash-summary-row">
            <span className="srv-dash-summary-k">Load 1m / ядро</span>
            <span className={'srv-dash-summary-v mono tone-' + loadTone(m.load[0], m.cores)}>
              {load1pct.toFixed(0)}%
            </span>
          </div>
          <div className="srv-dash-summary-row">
            <span className="srv-dash-summary-k">Ядер</span>
            <span className="srv-dash-summary-v mono">{m.cores}</span>
          </div>
        </div>
      </div>
      <div className="srv-load-block">
        <div className="srv-load-head">
          <span>Load average</span>
          <span className="srv-load-hint">100% = по одной задаче на каждое ядро</span>
        </div>
        <LoadRow label="1 мин" load={m.load[0]} cores={m.cores} />
        <LoadRow label="5 мин" load={m.load[1]} cores={m.cores} />
        <LoadRow label="15 мин" load={m.load[2]} cores={m.cores} />
      </div>
    </div>
  )
}

function Bar({ label, pct, sub }: { label: string; pct: number; sub: string }): JSX.Element {
  const t = tone(pct)
  return (
    <div className="mon-metric">
      <div className="mon-metric-head">
        <span>{label}</span>
        <span className="mon-sub">{sub}</span>
      </div>
      <div className="bar">
        <div className={'bar-fill tone-' + t} style={{ width: `${Math.min(100, pct)}%` }} />
      </div>
    </div>
  )
}

function MetricsCompact({ m }: { m: ServerMetrics }): JSX.Element {
  const memPct = m.memTotalKb > 0 ? (m.memUsedKb / m.memTotalKb) * 100 : 0
  return (
    <div className="mon-body docked">
      <Bar label="CPU" pct={m.cpuPct} sub={`${m.cpuPct}% · ${m.cores} ядр.`} />
      <Bar label="RAM" pct={memPct} sub={`${fmtKb(m.memUsedKb)} / ${fmtKb(m.memTotalKb)}`} />
      <Bar label="Диск /" pct={m.diskPct} sub={`${m.diskPct}%`} />
      <div className="mon-load">
        Load avg: <b>{m.load[0].toFixed(2)}</b> · {m.load[1].toFixed(2)} · {m.load[2].toFixed(2)}
      </div>
    </div>
  )
}

export function MonitorMetrics({
  sessionId,
  compact,
  variant = compact ? 'compact' : 'default'
}: {
  sessionId: string
  /** @deprecated — используй variant="compact" */
  compact?: boolean
  variant?: 'default' | 'compact' | 'dashboard'
}): JSX.Element {
  const [m, setM] = useState<ServerMetrics | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [fresh, setFresh] = useState(false)
  const aliveRef = useRef(true)

  useEffect(() => {
    aliveRef.current = true
    const tick = async (): Promise<void> => {
      try {
        const data = await window.api.session.monitor(sessionId)
        if (!aliveRef.current) return
        if (data.ok) {
          setM(data)
          setErr(null)
          setFresh(true)
          window.setTimeout(() => {
            if (aliveRef.current) setFresh(false)
          }, 600)
        } else setErr(data.error || 'Не удалось получить метрики')
      } catch (e) {
        if (aliveRef.current) setErr(errText(e))
      }
    }
    void tick()
    const id = window.setInterval(tick, 3000)
    return () => {
      aliveRef.current = false
      window.clearInterval(id)
    }
  }, [sessionId])

  const isDashboard = variant === 'dashboard'
  const isCompact = variant === 'compact'

  return (
    <div
      className={
        'ws-metrics' +
        (isCompact ? ' compact' : '') +
        (isDashboard ? ' dashboard' : '')
      }
    >
      <div className="ws-metrics-head">
        <span className="ws-metrics-title">
          <Icon name="monitor" size={14} />
          {isDashboard ? 'Загрузка сервера' : 'Ресурсы'}
        </span>
        <div className="ws-metrics-meta">
          {m && (
            <>
              <span className={'srv-live' + (fresh ? ' ping' : '')} title="Обновление каждые 3 с">
                live
              </span>
              <span className="mon-uptime" title="Uptime">
                <Icon name="arrow-up" size={11} /> {fmtUptime(m.uptimeSec)}
              </span>
            </>
          )}
        </div>
      </div>
      {err && <div className="hint ws-metrics-err">{err}</div>}
      {!err && !m && <div className="hint ws-metrics-err">Сбор метрик…</div>}
      {m && (isDashboard ? <MetricsDashboard m={m} /> : <MetricsCompact m={m} />)}
    </div>
  )
}
