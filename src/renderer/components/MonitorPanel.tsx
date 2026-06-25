import { useEffect, useRef, useState } from 'react'
import type { ServerMetrics } from '../../shared/types'
import { Icon } from './Icon'

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

function barColor(pct: number): string {
  return pct < 60 ? 'var(--green)' : pct < 85 ? '#e0af68' : 'var(--danger)'
}

function Bar({ label, pct, sub }: { label: string; pct: number; sub: string }): JSX.Element {
  return (
    <div className="mon-metric">
      <div className="mon-metric-head">
        <span>{label}</span>
        <span className="mon-sub">{sub}</span>
      </div>
      <div className="bar">
        <div className="bar-fill" style={{ width: `${Math.min(100, pct)}%`, background: barColor(pct) }} />
      </div>
    </div>
  )
}

export function MonitorPanel({ sessionId, onClose }: { sessionId: string; onClose: () => void }): JSX.Element {
  const [m, setM] = useState<ServerMetrics | null>(null)
  const [err, setErr] = useState<string | null>(null)
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
        } else setErr(data.error || 'Не удалось получить метрики')
      } catch (e) {
        if (aliveRef.current) setErr((e as Error).message)
      }
    }
    void tick()
    const id = setInterval(tick, 3000)
    return () => {
      aliveRef.current = false
      clearInterval(id)
    }
  }, [sessionId])

  const memPct = m && m.memTotalKb > 0 ? (m.memUsedKb / m.memTotalKb) * 100 : 0

  return (
    <>
      <div className="split-menu-backdrop" onClick={onClose} />
      <div className="monitor-panel">
        <div className="tunnel-menu-header">
          <span className="tunnel-menu-title">Мониторинг</span>
          {m && <span className="mon-uptime"><Icon name="arrow-up" size={11} /> {fmtUptime(m.uptimeSec)}</span>}
        </div>
        {err && <div className="hint" style={{ padding: '10px 12px' }}>{err}</div>}
        {!err && !m && <div className="hint" style={{ padding: '10px 12px' }}>Сбор метрик…</div>}
        {m && (
          <div className="mon-body">
            <Bar label="CPU" pct={m.cpuPct} sub={`${m.cpuPct}% · ${m.cores} ядр.`} />
            <Bar label="RAM" pct={memPct} sub={`${fmtKb(m.memUsedKb)} / ${fmtKb(m.memTotalKb)}`} />
            <Bar label="Диск /" pct={m.diskPct} sub={`${m.diskPct}%`} />
            <div className="mon-load">
              Load avg: <b>{m.load[0].toFixed(2)}</b> · {m.load[1].toFixed(2)} · {m.load[2].toFixed(2)}
            </div>
          </div>
        )}
      </div>
    </>
  )
}
