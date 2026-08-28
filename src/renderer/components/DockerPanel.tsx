import { useCallback, useEffect, useRef, useState } from 'react'
import type { DockerContainer, DockerAction } from '../../shared/types'
import { Icon } from './Icon'
import { DockerLogView, openDetachedLogsWindow, useLogsPanelResize } from './dockerLogs'
import { useCtrlWheelZoom } from '../useCtrlWheelZoom'

interface Props {
  /** SSH-сессия, на которой выполняем docker-команды и shell. */
  sessionId: string
  serverId?: string
  onClose: () => void
  /** В рельсе workspace — без попапа и backdrop. */
  docked?: boolean
  /** После «shell в контейнер» переключить вкладку на Terminal. */
  onGoTerminal?: () => void
}

export function DockerPanel({ sessionId, serverId, onClose, docked, onGoTerminal }: Props): JSX.Element {
  const [containers, setContainers] = useState<DockerContainer[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)
  const [logsFor, setLogsFor] = useState<DockerContainer | null>(null)
  const [logsText, setLogsText] = useState('')
  const [following, setFollowing] = useState(false)
  const logsForRef = useRef<string | null>(null)
  const skipCancelRef = useRef(false)
  const { size, onResizeDown } = useLogsPanelResize()
  const { zoom, ref: zoomRef, reset } = useCtrlWheelZoom('serein.logs.zoom')
  const LOADING = 'Загрузка логов…'

  const reload = useCallback(async () => {
    setLoading(true)
    const res = await window.api.docker.list(sessionId)
    setLoading(false)
    if (res.ok) {
      setContainers(res.containers ?? [])
      setError(null)
    } else setError(res.error ?? 'Ошибка')
  }, [sessionId])

  useEffect(() => {
    void reload()
  }, [reload])

  useEffect(() => {
    return window.api.docker.onLogs((p) => {
      if (p.sessionId !== sessionId) return
      const want = logsForRef.current
      if (!want) return
      if (p.containerId !== want && !p.containerId.startsWith(want)) return
      setLogsText((t) => {
        const base = t === LOADING ? '' : t
        const next = base + p.chunk
        return next.length > 400000 ? next.slice(-350000) : next
      })
    })
  }, [sessionId])

  useEffect(() => {
    return () => {
      if (!skipCancelRef.current) void window.api.docker.cancelLogs(sessionId)
    }
  }, [sessionId])

  const doAction = async (c: DockerContainer, action: DockerAction): Promise<void> => {
    if (action === 'remove' && !confirm(`Удалить контейнер «${c.name}»?`)) return
    setBusy(c.id)
    const res = await window.api.docker.action(sessionId, c.id, action)
    setBusy(null)
    if (!res.ok) setError(res.error ?? 'Ошибка')
    else void reload()
  }

  const openLogs = async (c: DockerContainer): Promise<void> => {
    if (logsForRef.current) void window.api.docker.cancelLogs(sessionId, logsForRef.current)
    logsForRef.current = c.id
    setLogsFor(c)
    setFollowing(true)
    setLogsText(LOADING)
    const res = await window.api.docker.logs(sessionId, c.id)
    if (logsForRef.current !== c.id) return
    setFollowing(false)
    if (res.ok) {
      setLogsText((t) => (t === LOADING ? res.logs || '(пусто)' : t))
    } else {
      setLogsText((t) => (t === LOADING ? `Ошибка: ${res.error}` : t))
    }
  }

  const stopLogs = (): void => {
    if (logsForRef.current) void window.api.docker.cancelLogs(sessionId, logsForRef.current)
    logsForRef.current = null
    setFollowing(false)
    setLogsFor(null)
  }

  const detachLogs = async (): Promise<void> => {
    if (!logsFor) return
    skipCancelRef.current = true
    if (logsForRef.current) void window.api.docker.cancelLogs(sessionId, logsForRef.current)
    logsForRef.current = null
    try {
      await openDetachedLogsWindow({
        sessionId,
        serverId,
        containerId: logsFor.id,
        name: logsFor.name,
        width: size.w,
        height: size.h
      })
    } catch (e) {
      skipCancelRef.current = false
      setFollowing(false)
      setLogsText(`Не удалось открепить: ${e instanceof Error ? e.message : String(e)}`)
      return
    }
    setFollowing(false)
    setLogsFor(null)
    if (!docked) onClose()
  }

  const openShell = (c: DockerContainer): void => {
    // Запускаем интерактивную оболочку контейнера прямо в активном терминале.
    window.api.session.write(sessionId, `docker exec -it ${c.id} sh\n`)
    onGoTerminal?.()
    if (!docked) onClose()
  }

  const running = (s: string): boolean => s === 'running' || s.startsWith('Up')

  return (
    <>
      {!docked && <div className="split-menu-backdrop" onClick={onClose} />}
      <div
        className={
          docked
            ? `ws-panel${logsFor ? ' is-logs' : ''}`
            : `docker-panel${logsFor ? ' is-logs' : ''}`
        }
        ref={logsFor ? zoomRef : undefined}
        style={logsFor && !docked ? { width: size.w, height: size.h, zoom } : undefined}
      >
        <div className={docked ? 'ws-head' : 'tunnel-menu-header'}>
          <span className={docked ? 'ws-head-title' : 'tunnel-menu-title'} title={logsFor ? logsFor.name : undefined}>
            {docked && <Icon name={logsFor ? 'logs' : 'docker'} size={15} />}
            {logsFor ? `Логи: ${logsFor.name}` : 'Docker'}
          </span>
          {logsFor ? (
            <>
              {following && (
                <button
                  className="mini"
                  onClick={() => {
                    if (logsForRef.current) void window.api.docker.cancelLogs(sessionId, logsForRef.current)
                    setFollowing(false)
                  }}
                >
                  Стоп
                </button>
              )}
              <button className="mini" title="Масштаб" onClick={reset}>{Math.round(zoom * 100)}%</button>
              <button className="mini" title="Открепить в отдельное окно" onClick={() => void detachLogs()}>
                <Icon name="external" size={14} />
              </button>
              <button className="mini" onClick={stopLogs}><Icon name="back" size={14} /> назад</button>
            </>
          ) : (
            <button className="mini" title="Обновить" onClick={() => void reload()}><Icon name="refresh" size={14} /></button>
          )}
        </div>

        {logsFor ? (
          <DockerLogView text={logsText} follow={following} />
        ) : (
          <div className="docker-list">
            {loading && <div className="hint" style={{ padding: '10px 12px' }}>Загрузка…</div>}
            {error && <div className="sftp-error" onClick={() => setError(null)}>{error}</div>}
            {!loading && !error && containers.length === 0 && (
              <div className={docked ? 'ws-empty' : 'hint'} style={docked ? undefined : { padding: '10px 12px' }}>
                Контейнеров нет.
              </div>
            )}
            {containers.map((c) => (
              <div key={c.id} className="docker-row">
                <span
                  className="docker-dot"
                  style={{ background: running(c.state) ? 'var(--green)' : 'var(--muted)' }}
                  title={c.status}
                />
                <div className="docker-info">
                  <div className="docker-name">{c.name}</div>
                  <div className="docker-image" title={c.image}>{c.image}</div>
                </div>
                <div className="docker-actions">
                  {running(c.state) ? (
                    <>
                      <button className="mini" title="Shell в контейнер" onClick={() => openShell(c)}><Icon name="shell" size={14} /></button>
                      <button className="mini" title="Логи" onClick={() => void openLogs(c)}><Icon name="logs" size={14} /></button>
                      <button className="mini" title="Перезапустить" disabled={busy === c.id} onClick={() => void doAction(c, 'restart')}><Icon name="restart" size={14} /></button>
                      <button className="mini" title="Остановить" disabled={busy === c.id} onClick={() => void doAction(c, 'stop')}><Icon name="stop" size={14} /></button>
                    </>
                  ) : (
                    <>
                      <button className="mini" title="Логи" onClick={() => void openLogs(c)}><Icon name="logs" size={14} /></button>
                      <button className="mini" title="Запустить" disabled={busy === c.id} onClick={() => void doAction(c, 'start')}><Icon name="play" size={14} /></button>
                      <button className="mini danger" title="Удалить" disabled={busy === c.id} onClick={() => void doAction(c, 'remove')}><Icon name="trash" size={14} /></button>
                    </>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
        {logsFor && !docked && (
          <>
            <div className="docker-resize e" onMouseDown={onResizeDown('e')} />
            <div className="docker-resize s" onMouseDown={onResizeDown('s')} />
            <div className="docker-resize se" onMouseDown={onResizeDown('se')} />
          </>
        )}
      </div>
    </>
  )
}
