import { useCallback, useEffect, useState } from 'react'
import type { DockerContainer, DockerAction } from '../../shared/types'
import { Icon } from './Icon'

interface Props {
  /** SSH-сессия, на которой выполняем docker-команды и shell. */
  sessionId: string
  onClose: () => void
}

export function DockerPanel({ sessionId, onClose }: Props): JSX.Element {
  const [containers, setContainers] = useState<DockerContainer[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)
  const [logsFor, setLogsFor] = useState<DockerContainer | null>(null)
  const [logsText, setLogsText] = useState('')

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

  const doAction = async (c: DockerContainer, action: DockerAction): Promise<void> => {
    if (action === 'remove' && !confirm(`Удалить контейнер «${c.name}»?`)) return
    setBusy(c.id)
    const res = await window.api.docker.action(sessionId, c.id, action)
    setBusy(null)
    if (!res.ok) setError(res.error ?? 'Ошибка')
    else void reload()
  }

  const openLogs = async (c: DockerContainer): Promise<void> => {
    setLogsFor(c)
    setLogsText('Загрузка логов…')
    const res = await window.api.docker.logs(sessionId, c.id)
    setLogsText(res.ok ? res.logs || '(пусто)' : `Ошибка: ${res.error}`)
  }

  const openShell = (c: DockerContainer): void => {
    // Запускаем интерактивную оболочку контейнера прямо в активном терминале.
    window.api.session.write(sessionId, `docker exec -it ${c.id} sh\n`)
    onClose()
  }

  const running = (s: string): boolean => s === 'running' || s.startsWith('Up')

  return (
    <>
      <div className="split-menu-backdrop" onClick={onClose} />
      <div className="docker-panel">
        <div className="tunnel-menu-header">
          <span className="tunnel-menu-title">{logsFor ? `Логи: ${logsFor.name}` : 'Docker'}</span>
          {logsFor ? (
            <button className="mini" onClick={() => setLogsFor(null)}><Icon name="back" size={14} /> назад</button>
          ) : (
            <button className="mini" title="Обновить" onClick={() => void reload()}><Icon name="refresh" size={14} /></button>
          )}
        </div>

        {logsFor ? (
          <pre className="docker-logs">{logsText}</pre>
        ) : (
          <div className="docker-list">
            {loading && <div className="hint" style={{ padding: '10px 12px' }}>Загрузка…</div>}
            {error && <div className="sftp-error" onClick={() => setError(null)}>{error}</div>}
            {!loading && !error && containers.length === 0 && (
              <div className="hint" style={{ padding: '10px 12px' }}>Контейнеров нет.</div>
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
      </div>
    </>
  )
}
