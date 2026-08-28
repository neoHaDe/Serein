import { useEffect, useMemo, useRef, useState, type MouseEvent, type ReactNode } from 'react'
import { applyUiTheme } from '../themes'
import { openAuxWindow, sanitizeWindowLabel } from '../auxWindows'
import { useCtrlWheelZoom } from '../useCtrlWheelZoom'
import { useWindowSnap } from '../windowSnap'
import { AuxDrag, WindowSysButtons, markAuxWindow } from './WindowChrome'

type LogKind = 'err' | 'warn' | 'info' | 'debug' | ''

function kindOfWord(w: string): LogKind {
  const u = w.replace(/^["']|["']$/g, '').toUpperCase()
  if (u === 'ERROR' || u === 'ERR' || u === 'FATAL' || u === 'CRITICAL' || u === 'CRIT' || u === 'PANIC') return 'err'
  if (u === 'WARN' || u === 'WARNING') return 'warn'
  if (u === 'DEBUG' || u === 'TRACE') return 'debug'
  if (u === 'INFO' || u === 'NOTICE') return 'info'
  return ''
}

function lineKind(line: string): LogKind {
  const m = line.match(
    /(?:^|[=\s/\[(])((?:ERROR|ERR|FATAL|CRITICAL|CRIT|PANIC|WARN(?:ING)?|INFO|NOTICE|DEBUG|TRACE))(?:\b|[\]\s)])/i
  )
  return m ? kindOfWord(m[1]) : ''
}

const LOG_TOKEN =
  /(\d{4}-\d{2}-\d{2}[T ][\d:.+-]+Z?)|(\b[A-Za-z_][\w.-]*=)("[^"]*"|[^\s]+)?|(\b(?:ERROR|WARN(?:ING)?|INFO|NOTICE|DEBUG|TRACE|FATAL|CRITICAL|PANIC)\b)|("[^"]*")/gi

function highlightLine(line: string): ReactNode {
  if (!line) return '\u00a0'
  const out: ReactNode[] = []
  let last = 0
  let k = 0
  LOG_TOKEN.lastIndex = 0
  let m: RegExpExecArray | null
  while ((m = LOG_TOKEN.exec(line))) {
    if (m.index > last) out.push(line.slice(last, m.index))
    if (m[1]) {
      out.push(
        <span key={k++} className="docker-log-ts">
          {m[1]}
        </span>
      )
    } else if (m[2]) {
      out.push(
        <span key={k++} className="docker-log-key">
          {m[2]}
        </span>
      )
      if (m[3]) {
        const lv = kindOfWord(m[3])
        const cls = lv ? `docker-log-lvl is-${lv}` : m[3].startsWith('"') ? 'docker-log-str' : 'docker-log-val'
        out.push(
          <span key={k++} className={cls}>
            {m[3]}
          </span>
        )
      }
    } else if (m[4]) {
      const lv = kindOfWord(m[4])
      out.push(
        <span key={k++} className={lv ? `docker-log-lvl is-${lv}` : undefined}>
          {m[4]}
        </span>
      )
    } else if (m[5]) {
      out.push(
        <span key={k++} className="docker-log-str">
          {m[5]}
        </span>
      )
    }
    last = m.index + m[0].length
  }
  if (last < line.length) out.push(line.slice(last))
  return out
}

export function DockerLogView({ text, follow }: { text: string; follow: boolean }): JSX.Element {
  const scroller = useRef<HTMLDivElement>(null)
  const lines = useMemo(() => text.split('\n'), [text])

  useEffect(() => {
    if (!follow) return
    const el = scroller.current
    if (el) el.scrollTop = el.scrollHeight
  }, [text, follow])

  return (
    <div className="docker-logs" ref={scroller}>
      {lines.map((line, i) => {
        const kind = lineKind(line)
        return (
          <div key={i} className={`docker-log-line${kind ? ` is-${kind}` : ''}`}>
            {highlightLine(line)}
          </div>
        )
      })}
    </div>
  )
}

export const LOGS_SIZE_KEY = 'serein.dockerLogs.size'

export function loadLogsSize(): { w: number; h: number } {
  try {
    const r = JSON.parse(localStorage.getItem(LOGS_SIZE_KEY) || '') as { w?: number; h?: number }
    if (typeof r.w === 'number' && typeof r.h === 'number' && r.w >= 360 && r.h >= 240) return { w: r.w, h: r.h }
  } catch {
    /* default */
  }
  return { w: 880, h: 560 }
}

export function useLogsPanelResize(): {
  size: { w: number; h: number }
  onResizeDown: (edge: string) => (e: MouseEvent) => void
} {
  const [size, setSize] = useState(loadLogsSize)
  const drag = useRef<{ x: number; y: number; w: number; h: number; edge: string } | null>(null)

  useEffect(() => {
    const move = (e: globalThis.MouseEvent): void => {
      const d = drag.current
      if (!d) return
      let w = d.w
      let h = d.h
      if (d.edge.includes('e')) w = d.w + (e.clientX - d.x)
      if (d.edge.includes('s')) h = d.h + (e.clientY - d.y)
      w = Math.min(Math.max(360, w), window.innerWidth - 16)
      h = Math.min(Math.max(240, h), window.innerHeight - 40)
      setSize({ w, h })
    }
    const up = (): void => {
      if (!drag.current) return
      drag.current = null
      setSize((s) => {
        localStorage.setItem(LOGS_SIZE_KEY, JSON.stringify(s))
        return s
      })
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
    return () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
    }
  }, [])

  const onResizeDown = (edge: string) => (e: MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    drag.current = { x: e.clientX, y: e.clientY, w: size.w, h: size.h, edge }
  }

  return { size, onResizeDown }
}

export function logsWindowLabel(sessionId: string, containerId: string): string {
  return "docker-logs-" + sanitizeWindowLabel(sessionId + "-" + containerId)
}

export async function openDetachedLogsWindow(opts: {
  sessionId: string
  serverId?: string
  containerId: string
  name: string
  width: number
  height: number
  x?: number
  y?: number
}): Promise<void> {
  await openAuxWindow({
    label: logsWindowLabel(opts.sessionId, opts.containerId),
    query: {
      dockerLogs: '1',
      sessionId: opts.sessionId,
      containerId: opts.containerId,
      name: opts.name
    },
    title: 'Логи · ' + opts.name,
    width: opts.width,
    height: opts.height,
    x: opts.x,
    y: opts.y,
    persist: opts.serverId
      ? { kind: 'dockerLogs', serverId: opts.serverId, containerId: opts.containerId, name: opts.name }
      : undefined
  })
}
const LOADING = 'Загрузка логов…'

/** Отдельное OS-окно логов (второй монитор, свой размер). */
export function DockerLogsWindow(): JSX.Element {
  const q = new URLSearchParams(window.location.search)
  const sessionId = q.get('sessionId') ?? ''
  const containerId = q.get('containerId') ?? ''
  const name = q.get('name') ?? containerId
  const [logsText, setLogsText] = useState(LOADING)
  const [following, setFollowing] = useState(true)
  const { zoom, ref, reset } = useCtrlWheelZoom('serein.logs.zoom')
  useWindowSnap()

  useEffect(() => {
    markAuxWindow()
    void window.api.settings.get().then((s) => {
      applyUiTheme(s.theme)
      document.documentElement.dataset.density = s.density ?? 'comfortable'
    })
  }, [])

  useEffect(() => {
    return window.api.docker.onLogs((p) => {
      if (p.sessionId !== sessionId) return
      if (p.containerId !== containerId && !p.containerId.startsWith(containerId)) return
      setLogsText((t) => {
        const base = t === LOADING ? '' : t
        const next = base + p.chunk
        return next.length > 400000 ? next.slice(-350000) : next
      })
    })
  }, [sessionId, containerId])

  useEffect(() => {
    if (!sessionId || !containerId) {
      setLogsText('Нет session/container в URL')
      setFollowing(false)
      return
    }
    let alive = true
    void window.api.docker.logs(sessionId, containerId).then((res) => {
      if (!alive) return
      setFollowing(false)
      if (res.ok) {
        setLogsText((t) => (t === LOADING ? res.logs || '(пусто)' : t))
      } else {
        setLogsText((t) => (t === LOADING ? `Ошибка: ${res.error}` : t))
      }
    })
    return () => {
      alive = false
      void window.api.docker.cancelLogs(sessionId, containerId)
    }
  }, [sessionId, containerId])

  return (
    <div className="docker-logs-window" ref={ref} style={{ zoom }}>
      <div className="tunnel-menu-header aux-win-header">
        <AuxDrag>
          <span className="tunnel-menu-title" title={name}>
            Логи: {name}
          </span>
        </AuxDrag>
        <div className="aux-win-actions">
          <button className="mini" title="Сброс масштаба" onClick={reset}>{Math.round(zoom * 100)}%</button>
          {following && (
            <button
              type="button"
              className="mini"
              onClick={(e) => {
                e.preventDefault()
                e.stopPropagation()
                void window.api.docker.cancelLogs(sessionId, containerId)
                setFollowing(false)
              }}
            >
              Стоп
            </button>
          )}
          <WindowSysButtons />
        </div>
      </div>
      <DockerLogView text={logsText} follow={following} />
    </div>
  )
}
