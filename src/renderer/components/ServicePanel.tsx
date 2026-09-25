import { useCallback, useEffect, useMemo, useState } from 'react'
import type { WorkspaceService } from '../../shared/types'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'
import { confirmAction } from '../confirmDialog'

type ServiceAction = 'start' | 'stop' | 'restart'

export function ServicePanel({
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
  const [rows, setRows] = useState<WorkspaceService[]>([])
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [filter, setFilter] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  // Действие, которому не хватило прав: ждёт пароль sudo. Пароль живёт только в поле формы
  // и стирается сразу после попытки.
  const [pending, setPending] = useState<{ row: WorkspaceService; action: ServiceAction } | null>(null)
  const [sudo, setSudo] = useState('')

  const reload = useCallback(async () => {
    setLoading(true)
    const res = await window.api.workspace.services(sessionId)
    setLoading(false)
    setNote(res.note ?? null)
    if (res.ok) {
      setRows(res.rows ?? [])
      setError(null)
    } else setError(res.error ?? 'Список служб недоступен')
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

  const run = async (row: WorkspaceService, action: ServiceAction, sudoPassword?: string): Promise<void> => {
    setBusy(row.name + action)
    setError(null)
    const res = await window.api.workspace.serviceAction(sessionId, row.name, action, sudoPassword)
    setBusy(null)
    setSudo('')
    setPending(res.needSudo ? { row, action } : null)
    if (!res.ok) setError(res.error ?? `Служба ${row.name}: действие не удалось`)
    else void reload()
  }

  const act = async (row: WorkspaceService, action: ServiceAction): Promise<void> => {
    const verb = action === 'start' ? 'запустить' : action === 'stop' ? 'остановить' : 'перезапустить'
    if (!(await confirmAction(`${verb[0]!.toUpperCase() + verb.slice(1)} ${row.name}?`))) return
    setPending(null)
    await run(row, action)
  }

  const detach = async (): Promise<void> => {
    if (!panelTitle) return
    await openDetachedWorkspace({ tool: 'services', sessionId, title: panelTitle })
    onDetached?.()
  }

  return (
    <div className={'ws-panel' + (fill ? ' fill' : '')}>
      <div className="ws-head">
        <span className="ws-head-title"><Icon name="settings" size={15} /> Сервисы</span>
        <div style={{ display: 'flex', gap: 6 }}>
          {panelTitle && onDetached && <WsDetachButton onClick={detach} />}
          <button className="mini" title="Обновить" onClick={() => void reload()}>
            <Icon name="refresh" size={14} />
          </button>
        </div>
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
      {pending && (
        <form
          className="ws-sudo"
          onSubmit={(e) => {
            e.preventDefault()
            if (sudo) void run(pending.row, pending.action, sudo)
          }}
        >
          <input
            type="password"
            autoFocus
            placeholder="Пароль sudo"
            value={sudo}
            onChange={(e) => setSudo(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setPending(null)
            }}
          />
          <button className="primary" type="submit" disabled={!sudo || busy !== null}>
            {busy ? '…' : 'Выполнить'}
          </button>
          <button className="secondary" type="button" onClick={() => setPending(null)}>
            Отмена
          </button>
        </form>
      )}
      {note && <div className="agent-hint">{note}</div>}
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
                  <tr key={r.name}>
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
