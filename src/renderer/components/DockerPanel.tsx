import { useCallback, useEffect, useRef, useState } from 'react'
import type { DockerContainer, DockerAction } from '../../shared/types'
import { Icon } from './Icon'
import { DockerLogView, openDetachedLogsWindow, useLogsPanelResize } from './dockerLogs'
import { useCtrlWheelZoom } from '../useCtrlWheelZoom'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'

interface Props {
  /** SSH-сессия, на которой выполняем docker-команды и shell. */
  sessionId: string
  serverId?: string
  panelTitle?: string
  onClose: () => void
  /** В рельсе workspace — без попапа и backdrop. */
  docked?: boolean
  fill?: boolean
  /** После «shell в контейнер» переключить вкладку на Terminal. */
  onGoTerminal?: () => void
  /** После открепления панели в отдельное окно. */
  onDetached?: () => void
}

function DockerPropsModal({
  container: c,
  onClose
}: {
  container: DockerContainer
  onClose: () => void
}): JSX.Element {
  const rows = [
    { k: 'Имя', v: c.name },
    { k: 'ID', v: c.id },
    { k: 'Образ', v: c.image },
    { k: 'State', v: c.state },
    { k: 'Status', v: c.status }
  ]
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal sftp-props-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Свойства контейнера</h2>
        <dl className="sftp-props">
          {rows.map((r) => (
            <div key={r.k} className="sftp-props-row">
              <dt>{r.k}</dt>
              <dd title={r.v}>{r.v || '—'}</dd>
            </div>
          ))}
        </dl>
        <div className="modal-actions">
          <button className="primary" onClick={onClose}>
            Закрыть
          </button>
        </div>
      </div>
    </div>
  )
}

export function DockerPanel({
  sessionId,
  serverId,
  panelTitle,
  onClose,
  docked,
  fill,
  onGoTerminal,
  onDetached
}: Props): JSX.Element {
  const [containers, setContainers] = useState<DockerContainer[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)
  const [logsFor, setLogsFor] = useState<DockerContainer | null>(null)
  const [logsText, setLogsText] = useState('')
  const [following, setFollowing] = useState(false)
  const [ctx, setCtx] = useState<{ x: number; y: number; container: DockerContainer } | null>(null)
  const [propsFor, setPropsFor] = useState<DockerContainer | null>(null)
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

  const detachPanel = async (): Promise<void> => {
    if (!panelTitle) return
    await openDetachedWorkspace({ tool: 'docker', sessionId, serverId, title: panelTitle })
    onDetached?.()
  }

  const running = (s: string): boolean => s === 'running' || s.startsWith('Up')

  return (
    <>
      {!docked && <div className="split-menu-backdrop" onClick={onClose} />}
      <div
        className={
          docked
            ? `ws-panel${fill ? ' fill' : ''}${logsFor ? ' is-logs' : ''}`
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
            <>
              {docked && panelTitle && onDetached && !fill && <WsDetachButton onClick={detachPanel} />}
              <button className="mini" title="Обновить" onClick={() => void reload()}><Icon name="refresh" size={14} /></button>
            </>
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
              <div
                key={c.id}
                className="docker-row"
                title={c.status}
                onContextMenu={(e) => {
                  e.preventDefault()
                  setCtx({ x: e.clientX, y: e.clientY, container: c })
                }}
              >
                <span
                  className="docker-dot"
                  style={{ background: running(c.state) ? 'var(--green)' : 'var(--muted)' }}
                />
                <div className="docker-info">
                  <div className="docker-name">{c.name}</div>
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
      {ctx && (
        <>
          <div className="split-menu-backdrop" onClick={() => setCtx(null)} />
          <div
            className="sftp-ctx-menu"
            style={{ left: ctx.x, top: ctx.y }}
            onMouseDown={(e) => e.stopPropagation()}
          >
            <button
              type="button"
              className="sftp-ctx-item"
              onClick={() => {
                setPropsFor(ctx.container)
                setCtx(null)
              }}
            >
              Свойства
            </button>
          </div>
        </>
      )}
      {propsFor && <DockerPropsModal container={propsFor} onClose={() => setPropsFor(null)} />}
    </>
  )
}
