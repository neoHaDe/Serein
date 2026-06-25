import { useEffect, useRef, useState } from 'react'
import type { ServerConfig } from '../../shared/types'
import type { PaneLeaf } from '../paneTree'
import { Icon } from './Icon'

interface Props {
  leaf: PaneLeaf | undefined
  server: ServerConfig | undefined
  broadcast: boolean
  /** Сколько сессий получит broadcast-ввод (панели текущей вкладки). */
  broadcastTargets?: number
  editor?: { remotePath: string; dirty: boolean }
}

const statusText: Record<string, string> = {
  connecting: 'Подключение…',
  connected: 'Подключено',
  closed: 'Закрыто',
  error: 'Ошибка'
}
const statusColor: Record<string, string> = {
  connecting: '#e0af68',
  connected: '#9ece6a',
  closed: '#565f89',
  error: '#f7768e'
}

export function StatusBar({ leaf, server, broadcast, broadcastTargets, editor }: Props): JSX.Element {
  const [latency, setLatency] = useState<number | null>(null)
  const sessionId = leaf?.kind === 'ssh' && leaf.status === 'connected' ? leaf.sessionId : undefined
  const timerRef = useRef<ReturnType<typeof setInterval>>()

  useEffect(() => {
    setLatency(null)
    if (!sessionId) return
    let alive = true
    const measure = async (): Promise<void> => {
      const ms = await window.api.session.ping(sessionId)
      if (alive) setLatency(ms)
    }
    measure()
    timerRef.current = setInterval(measure, 5000)
    return () => {
      alive = false
      clearInterval(timerRef.current)
    }
  }, [sessionId])

  if (editor) {
    return (
      <div className="statusbar">
        <span className="sb-item"><Icon name="editor" size={13} /> Редактор</span>
        <span className="sb-item sb-muted">{editor.remotePath}</span>
        <span className="sb-spacer" />
        <span className="sb-item" style={{ color: editor.dirty ? '#e0af68' : '#9ece6a' }}>
          {editor.dirty ? '● Несохранено' : <><Icon name="check" size={13} /> Сохранено</>}
        </span>
      </div>
    )
  }

  if (!leaf) {
    return (
      <div className="statusbar">
        <span className="sb-muted">Нет активной панели</span>
      </div>
    )
  }

  const isSsh = leaf.kind === 'ssh'
  const latColor = latency == null ? '#565f89' : latency < 80 ? '#9ece6a' : latency < 200 ? '#e0af68' : '#f7768e'

  return (
    <div className="statusbar">
      <span className="sb-item">
        <span className="status-dot" style={{ background: statusColor[leaf.status] }} />
        {statusText[leaf.status] || leaf.status}
      </span>
      {isSsh && server && (
        <span className="sb-item sb-muted">
          {server.username}@{server.host}:{server.port}
        </span>
      )}
      {isSsh && !server && <span className="sb-item sb-muted">{leaf.title}</span>}
      {!isSsh && <span className="sb-item sb-muted"><Icon name="desktop" size={13} /> Локальный терминал</span>}
      {leaf.status === 'error' && leaf.statusMsg && (
        <span className="sb-item sb-error" title={leaf.statusMsg}>
          {leaf.statusMsg}
        </span>
      )}
      <span className="sb-spacer" />
      {broadcast && (
        <span className="sb-item sb-broadcast" title="Broadcast активен — ввод дублируется в панели текущей вкладки">
          <Icon name="broadcast" size={13} /> Broadcast{broadcastTargets ? ` → ${broadcastTargets}` : ''}
        </span>
      )}
      {sessionId && (
        <span className="sb-item" style={{ color: latColor }} title="Задержка соединения (round-trip)">
          <Icon name="bolt" size={12} /> {latency == null ? '…' : `${latency} мс`}
        </span>
      )}
    </div>
  )
}
