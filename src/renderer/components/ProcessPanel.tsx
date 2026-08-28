import { useCallback, useEffect, useMemo, useState } from 'react'
import type { WorkspaceProcess } from '../../shared/types'
import { Icon } from './Icon'
import { MonitorMetrics } from './MonitorMetrics'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'

export function ProcessPanel({
  sessionId,
  panelTitle,
  onDetached,
  fill
}: {
  sessionId: string
  panelTitle?: string
  onDetached?: () => void
  fill?: boolean
}): JSX.Element {
  const [rows, setRows] = useState<WorkspaceProcess[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [filter, setFilter] = useState('')
  const [busy, setBusy] = useState<number | null>(null)

  const reload = useCallback(async () => {
    setLoading(true)
    const res = await window.api.workspace.processes(sessionId)
    setLoading(false)
    if (res.ok) {
      setRows(res.rows ?? [])
      setError(null)
    } else setError(res.error ?? 'Не удалось получить процессы')
  }, [sessionId])

  useEffect(() => {
    void reload()
  }, [reload])

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase()
    if (!q) return rows
    return rows.filter(
      (r) =>
        r.cmd.toLowerCase().includes(q) ||
        r.user.toLowerCase().includes(q) ||
        String(r.pid).includes(q)
    )
  }, [rows, filter])

  const kill = async (row: WorkspaceProcess): Promise<void> => {
    if (!confirm(`Завершить процесс ${row.pid} (${row.cmd})?`)) return
    setBusy(row.pid)
    const res = await window.api.workspace.kill(sessionId, row.pid)
    setBusy(null)
    if (!res.ok) setError(res.error ?? 'kill не удался')
    else void reload()
  }

  const detach = async (): Promise<void> => {
    if (!panelTitle) return
    await openDetachedWorkspace({ tool: 'processes', sessionId, title: panelTitle })
    onDetached?.()
  }

  return (
    <div className={'ws-panel' + (fill ? ' fill' : '')}>
      <div className="ws-head">
        <span className="ws-head-title"><Icon name="list" size={15} /> Процессы</span>
        <div style={{ display: 'flex', gap: 6 }}>
          {panelTitle && onDetached && <WsDetachButton onClick={detach} />}
          <button className="mini" title="Обновить" onClick={() => void reload()}>
            <Icon name="refresh" size={14} />
          </button>
        </div>
      </div>
      <MonitorMetrics sessionId={sessionId} compact />
      <div className="ws-toolbar">
        <input
          className="search"
          placeholder="Фильтр: имя, user, pid…"
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
                <th>PID</th>
                <th>User</th>
                <th>CPU</th>
                <th>MEM</th>
                <th>STAT</th>
                <th>CMD</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {filtered.length === 0 && (
                <tr>
                  <td colSpan={7} className="hint" style={{ padding: '12px' }}>
                    Ничего не найдено.
                  </td>
                </tr>
              )}
              {filtered.map((r) => (
                <tr key={r.pid}>
                  <td className="mono">{r.pid}</td>
                  <td>{r.user}</td>
                  <td className="mono">{r.cpu.toFixed(1)}</td>
                  <td className="mono">{r.mem.toFixed(1)}</td>
                  <td className="mono">{r.stat}</td>
                  <td className="ws-cmd" title={r.cmd}>
                    {r.cmd}
                  </td>
                  <td>
                    <button
                      className="mini danger"
                      disabled={busy === r.pid || r.pid <= 1}
                      title="kill"
                      onClick={() => void kill(r)}
                    >
                      Kill
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
