import { useEffect, useState } from 'react'
import type { ServerConfig, TunnelConfig, TunnelStatus } from '../../shared/types'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'

function tunnelDesc(t: TunnelConfig): string {
  if (t.type === 'local') return `127.0.0.1:${t.localPort} → ${t.remoteHost}:${t.remotePort}`
  if (t.type === 'remote') return `сервер:${t.remotePort} → :${t.localPort}`
  return `SOCKS5 :${t.localPort}`
}

interface Props {
  sessionId: string
  server: ServerConfig | undefined
  panelTitle?: string
  onClose: () => void
  onEditServer: () => void
  docked?: boolean
  fill?: boolean
  onDetached?: () => void
}

export function TunnelPanel({
  sessionId,
  server,
  panelTitle,
  onClose,
  onEditServer,
  docked,
  fill,
  onDetached
}: Props): JSX.Element {
  const [statuses, setStatuses] = useState<Map<string, TunnelStatus>>(new Map())
  const [opening, setOpening] = useState<Set<string>>(new Set())

  useEffect(() => {
    window.api.tunnel.listStatus(sessionId).then((list) => {
      setStatuses(new Map(list.map((s) => [s.tunnelId, s])))
    })
    return window.api.tunnel.onStatus((s) => {
      if (s.sessionId !== sessionId) return
      if (s.active || s.error) {
        setOpening((prev) => {
          if (!prev.has(s.tunnelId)) return prev
          const next = new Set(prev)
          next.delete(s.tunnelId)
          return next
        })
      }
      setStatuses((prev) => {
        const next = new Map(prev)
        next.set(s.tunnelId, s)
        return next
      })
    })
  }, [sessionId])

  const cfgs = server?.tunnels ?? []
  const edit = (): void => {
    if (!docked) onClose()
    onEditServer()
  }

  const detach = async (): Promise<void> => {
    if (!panelTitle) return
    await openDetachedWorkspace({ tool: 'tunnels', sessionId, serverId: server?.id, title: panelTitle })
    onDetached?.()
  }

  return (
    <>
      {!docked && <div className="split-menu-backdrop" onClick={onClose} />}
      <div className={(docked ? 'ws-panel' : 'tunnel-menu') + (fill ? ' fill' : '')}>
        <div className={docked ? 'ws-head' : 'tunnel-menu-header'}>
          <span className={docked ? 'ws-head-title' : 'tunnel-menu-title'}>
            {docked && <Icon name="tunnel" size={15} />}
            Туннели
          </span>
          <div style={{ display: 'flex', gap: 6 }}>
            {docked && panelTitle && onDetached && !fill && <WsDetachButton onClick={detach} />}
            {server && (
              <button className="mini" title="Настроить туннели в форме сервера" onClick={edit}>
                <Icon name="settings" size={14} />
              </button>
            )}
          </div>
        </div>
        {cfgs.length === 0 ? (
          <div className={docked ? 'ws-empty' : undefined} style={docked ? undefined : { padding: '10px 12px' }}>
            <div className="hint">Туннели не настроены.</div>
            {server && (
              <button className="primary" onClick={edit}>
                Редактировать сервер
              </button>
            )}
          </div>
        ) : (
          <div className="tunnel-list">
            {cfgs.map((t) => {
              const st = statuses.get(t.id)
              const active = st?.active ?? false
              const error = st?.error
              const isOpening = opening.has(t.id)
              return (
                <div key={t.id} className="tunnel-row">
                  <span
                    className="tunnel-dot"
                    style={{
                      background: active
                        ? 'var(--green)'
                        : error
                          ? 'var(--danger)'
                          : isOpening
                            ? 'var(--yellow, #e0af68)'
                            : 'var(--muted)'
                    }}
                    title={error ?? (active ? 'Активен' : isOpening ? 'Открывается…' : 'Неактивен')}
                  />
                  <span className="tunnel-type-badge">
                    {t.type === 'local' ? 'L' : t.type === 'remote' ? 'R' : 'D'}
                  </span>
                  <span className="tunnel-desc" title={error}>
                    {tunnelDesc(t)}
                  </span>
                  <button
                    className="mini"
                    onClick={async () => {
                      if (active || isOpening) {
                        setOpening((prev) => {
                          const next = new Set(prev)
                          next.delete(t.id)
                          return next
                        })
                        await window.api.tunnel.close(sessionId, t.id)
                      } else {
                        setOpening((prev) => new Set(prev).add(t.id))
                        try {
                          await window.api.tunnel.open(sessionId, t.id)
                        } catch {
                          /* отмена или ошибка — статус придёт событием */
                        }
                        setOpening((prev) => {
                          const next = new Set(prev)
                          next.delete(t.id)
                          return next
                        })
                      }
                    }}
                  >
                    {active ? 'Стоп' : isOpening ? 'Отмена' : 'Старт'}
                  </button>
                </div>
              )
            })}
          </div>
        )}
      </div>
    </>
  )
}
