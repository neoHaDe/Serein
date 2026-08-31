import { useEffect, useRef, useState } from 'react'
import type { ServerMetrics, WorkspaceTool } from '../../shared/types'
import { Icon } from './Icon'
import { errText } from '../errText'

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`
  const kb = n / 1024
  if (kb < 1024) return `${kb.toFixed(1)} KiB`
  const mb = kb / 1024
  if (mb < 1024) return `${mb.toFixed(1)} MiB`
  return `${(mb / 1024).toFixed(2)} GiB`
}

function fmtRate(bps: number): string {
  if (bps < 1024) return `${bps.toFixed(0)} B/s`
  const k = bps / 1024
  if (k < 1024) return `${k.toFixed(1)} KiB/s`
  return `${(k / 1024).toFixed(1)} MiB/s`
}

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

function OverviewCards({
  m,
  netRxRate,
  netTxRate,
  onGoTool
}: {
  m: ServerMetrics
  netRxRate: number | null
  netTxRate: number | null
  onGoTool?: (tool: WorkspaceTool) => void
}): JSX.Element {
  const failed = m.failedServices
  const svcTone =
    failed === undefined ? 'muted' : failed === 0 ? 'ok' : 'bad'
  const svcLabel =
    failed === undefined
      ? 'systemctl недоступен'
      : failed === 0
        ? 'Все сервисы в порядке'
        : `${failed} упало`

  let dockerLabel = 'Docker не установлен'
  let dockerTone: 'muted' | 'ok' | 'warn' = 'muted'
  if (m.dockerAvailable) {
    const run = m.dockerRunning ?? 0
    const stop = m.dockerStopped ?? 0
    dockerLabel = `${run} работает · ${stop} остановлено`
    dockerTone = stop > 0 ? 'warn' : 'ok'
  }

  const netDetail =
    m.netIface && (netRxRate !== null || netTxRate !== null)
      ? `${m.netIface}: ↓ ${netRxRate !== null ? fmtRate(netRxRate) : '—'} · ↑ ${netTxRate !== null ? fmtRate(netTxRate) : '—'}`
      : m.netRxBytes !== undefined
        ? `${m.netIface ?? 'iface'}: ↓ ${fmtBytes(m.netRxBytes)} · ↑ ${fmtBytes(m.netTxBytes ?? 0)}`
        : 'Интерфейс не определён'

  return (
    <div className="srv-overview-cards">
      {(m.os || m.kernel) && (
        <div className="srv-overview-card">
          <span className="srv-overview-k">ОС и ядро</span>
          <span className="srv-overview-v">{m.os ?? 'Linux'}</span>
          {m.kernel && <span className="srv-overview-sub mono">{m.kernel}</span>}
        </div>
      )}
      {m.procCount !== undefined && (
        <div className="srv-overview-card">
          <span className="srv-overview-k">Процессы</span>
          <span className="srv-overview-v mono">{m.procCount}</span>
          {onGoTool && (
            <button type="button" className="srv-overview-link" onClick={() => onGoTool('processes')}>
              Открыть список
            </button>
          )}
        </div>
      )}
      <div className="srv-overview-card">
        <span className="srv-overview-k">Сеть</span>
        <span className="srv-overview-v srv-overview-net">{netDetail}</span>
      </div>
      <div className={'srv-overview-card tone-' + svcTone}>
        <span className="srv-overview-k">Systemd</span>
        <span className="srv-overview-v">{svcLabel}</span>
        {onGoTool && failed !== undefined && (
          <button type="button" className="srv-overview-link" onClick={() => onGoTool('services')}>
            Сервисы
          </button>
        )}
      </div>
      <div className={'srv-overview-card tone-' + dockerTone}>
        <span className="srv-overview-k">Docker</span>
        <span className="srv-overview-v">{dockerLabel}</span>
        {onGoTool && m.dockerAvailable && (
          <button type="button" className="srv-overview-link" onClick={() => onGoTool('docker')}>
            Контейнеры
          </button>
        )}
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
  variant = compact ? 'compact' : 'default',
  showOverviewCards,
  onGoTool
}: {
  sessionId: string
  /** @deprecated — используй variant="compact" */
  compact?: boolean
  variant?: 'default' | 'compact' | 'dashboard'
  showOverviewCards?: boolean
  onGoTool?: (tool: WorkspaceTool) => void
}): JSX.Element {
  const [m, setM] = useState<ServerMetrics | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [fresh, setFresh] = useState(false)
  const [netRxRate, setNetRxRate] = useState<number | null>(null)
  const [netTxRate, setNetTxRate] = useState<number | null>(null)
  const aliveRef = useRef(true)
  const prevNetRef = useRef<{ rx?: number; tx?: number; at: number } | null>(null)

  useEffect(() => {
    aliveRef.current = true
    prevNetRef.current = null
    setNetRxRate(null)
    setNetTxRate(null)
    const tick = async (): Promise<void> => {
      try {
        const data = await window.api.session.monitor(sessionId)
        if (!aliveRef.current) return
        if (data.ok) {
          const now = Date.now()
          if (data.netRxBytes !== undefined && data.netTxBytes !== undefined) {
            const prev = prevNetRef.current
            if (prev?.rx !== undefined && prev.tx !== undefined) {
              const dt = (now - prev.at) / 1000
              if (dt > 0.2) {
                setNetRxRate(Math.max(0, (data.netRxBytes - prev.rx) / dt))
                setNetTxRate(Math.max(0, (data.netTxBytes - prev.tx) / dt))
              }
            }
            prevNetRef.current = { rx: data.netRxBytes, tx: data.netTxBytes, at: now }
          }
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
          {m?.os && isDashboard && (
            <span className="mon-os" title={m.kernel}>
              {m.os}
            </span>
          )}
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
      {m && showOverviewCards && (
        <OverviewCards m={m} netRxRate={netRxRate} netTxRate={netTxRate} onGoTool={onGoTool} />
      )}
    </div>
  )
}
