import { useCallback, useEffect, useMemo, useState } from 'react'
import type { WorkspaceService } from '../../shared/types'
import { Icon } from './Icon'

export function ServicePanel({ sessionId }: { sessionId: string }): JSX.Element {
  const [rows, setRows] = useState<WorkspaceService[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [filter, setFilter] = useState('')
  const [busy, setBusy] = useState<string | null>(null)

  const reload = useCallback(async () => {
    setLoading(true)
    const res = await window.api.workspace.services(sessionId)
    setLoading(false)
    if (res.ok) {
      setRows(res.rows ?? [])
      setError(null)
    } else setError(res.error ?? 'systemctl недоступен')
  }, [sessionId])

  useEffect(() => {
    void reload()
  }, [reload])

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase()
    if (!q) return rows
    return rows.filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        r.desc.toLowerCase().includes(q) ||
        r.active.toLowerCase().includes(q) ||
        r.sub.toLowerCase().includes(q)
    )
  }, [rows, filter])

  const act = async (row: WorkspaceService, action: 'start' | 'stop' | 'restart'): Promise<void> => {
    const verb = action === 'start' ? 'запустить' : action === 'stop' ? 'остановить' : 'перезапустить'
    if (!confirm(`${verb[0]!.toUpperCase() + verb.slice(1)} ${row.name}?`)) return
    setBusy(row.name + action)
    const res = await window.api.workspace.serviceAction(sessionId, row.name, action)
    setBusy(null)
    if (!res.ok) setError(res.error ?? `systemctl ${action} не удался`)
    else void reload()
  }

  return (
    <div className="ws-panel">
      <div className="ws-head">
        <span className="ws-head-title"><Icon name="settings" size={15} /> Сервисы</span>
        <button className="mini" title="Обновить" onClick={() => void reload()}>
          <Icon name="refresh" size={14} />
        </button>
      </div>
      <div className="ws-toolbar">
        <input
          className="search"
          placeholder="Фильтр: имя, описание, статус…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      </div>
      {error && (
        <div className="sftp-error" onClick={() => setError(null)}>
          {error}
        </div>
      )}
      {loading && <div className="hint" style={{ padding: '10px 12px' }}>Загрузка…</div>}
      {!loading && (
        <div className="ws-table-wrap">
          <table className="ws-table">
            <thead>
              <tr>
                <th>Имя</th>
                <th>Active</th>
                <th>Sub</th>
                <th>Описание</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {filtered.length === 0 && (
                <tr>
                  <td colSpan={5} className="hint" style={{ padding: '12px' }}>
                    Ничего не найдено.
                  </td>
                </tr>
              )}
              {filtered.map((r) => {
                const running = r.active === 'active'
                const failed = r.active === 'failed' || r.sub === 'failed'
                return (
                  <tr key={r.unit}>
                    <td className="mono">{r.name}</td>
                    <td>
                      <span
                        className="ws-badge"
                        style={{
                          color: running ? 'var(--green)' : failed ? 'var(--danger)' : 'var(--muted)'
                        }}
                      >
                        {r.active}
                      </span>
                    </td>
                    <td className="mono">{r.sub}</td>
                    <td className="ws-cmd" title={r.desc}>
                      {r.desc}
                    </td>
                    <td className="ws-row-actions">
                      {running ? (
                        <>
                          <button
                            className="mini"
                            disabled={busy === r.name + 'restart'}
                            title="Перезапустить"
                            onClick={() => void act(r, 'restart')}
                          >
                            <Icon name="restart" size={13} />
                          </button>
                          <button
                            className="mini"
                            disabled={busy === r.name + 'stop'}
                            title="Остановить"
                            onClick={() => void act(r, 'stop')}
                          >
                            <Icon name="stop" size={13} />
                          </button>
                        </>
                      ) : (
                        <button
                          className="mini"
                          disabled={busy === r.name + 'start'}
                          title="Запустить"
                          onClick={() => void act(r, 'start')}
                        >
                          <Icon name="play" size={13} />
                        </button>
                      )}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
