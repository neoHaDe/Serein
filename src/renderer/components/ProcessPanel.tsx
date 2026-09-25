import { useCallback, useEffect, useMemo, useState } from 'react'
import type { WorkspaceProcess } from '../../shared/types'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'
import { metricView } from '../processMetric'
import { loadSort, matchesQuery, nextSort, saveSort, sortRows, type SortDir } from '../tableSort'
import { SortHeader } from './SortHeader'
import { confirmAction } from '../confirmDialog'

type ProcessKey = 'pid' | 'user' | 'cpu' | 'mem' | 'stat' | 'cmd'

const COLUMNS: { key: ProcessKey; label: string; firstDir: SortDir }[] = [
  { key: 'pid', label: 'PID', firstDir: 'asc' },
  { key: 'user', label: 'User', firstDir: 'asc' },
  // Числа сначала по убыванию: щёлкают по CPU и памяти, чтобы увидеть самых прожорливых.
  { key: 'cpu', label: 'CPU', firstDir: 'desc' },
  { key: 'mem', label: 'MEM', firstDir: 'desc' },
  { key: 'stat', label: 'STAT', firstDir: 'asc' },
  { key: 'cmd', label: 'CMD', firstDir: 'asc' }
]
const KEYS = COLUMNS.map((c) => c.key)
const SORT_STORAGE = 'serein.sort.processes'
/**
 * Столько процессов отдаёт сервер, см. `PS_CMD` в `workspace.rs`. Раньше приходили первые 80
 * по процессору, и сортировка по памяти упорядочила бы только их: процесс с гигабайтами при
 * нулевой загрузке в таблицу не попадал вовсе.
 */
const PROCESS_CAP = 2000

function processCell(r: WorkspaceProcess, key: ProcessKey): number | string | null {
  switch (key) {
    case 'pid':
      return r.pid
    case 'user':
      return r.user
    case 'cpu':
      return r.cpu
    case 'mem':
      return r.mem
    case 'stat':
      return r.stat
    case 'cmd':
      return r.cmd
  }
}

/** Ячейка с долей в процентах. Что именно показывать - решает `processMetric.ts`. */
function MetricCell({ value, kind }: { value: number | null; kind?: 'mem' }): JSX.Element {
  const v = metricView(value)
  if (v.barPct === null) {
    return (
      <td className="mono hint" title={v.title}>
        {v.text}
      </td>
    )
  }
  return (
    <td className="mono ws-metric-cell">
      <span className="ws-metric-num">{v.text}</span>
      <span
        className={'ws-metric-bar' + (kind === 'mem' ? ' mem' : '')}
        style={{ width: `${v.barPct}%` }}
      />
    </td>
  )
}

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
  const [note, setNote] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [filter, setFilter] = useState('')
  const [busy, setBusy] = useState<number | null>(null)
  const [sort, setSort] = useState(() => loadSort<ProcessKey>(SORT_STORAGE, KEYS, { key: 'cpu', dir: 'desc' }))

  const reload = useCallback(async () => {
    setLoading(true)
    const res = await window.api.workspace.processes(sessionId)
    setLoading(false)
    setNote(res.note ?? null)
    if (res.ok) {
      setRows(res.rows ?? [])
      setError(null)
    } else setError(res.error ?? 'Не удалось получить процессы')
  }, [sessionId])

  useEffect(() => {
    void reload()
  }, [reload])

  const shown = useMemo(
    () =>
      sortRows(
        rows.filter((r) => matchesQuery([r.pid, r.user, r.stat, r.cmd], filter)),
        sort,
        processCell
      ),
    [rows, filter, sort]
  )

  const onSort = (key: ProcessKey): void => {
    const col = COLUMNS.find((c) => c.key === key)
    const next = nextSort(sort, key, col?.firstDir ?? 'asc')
    setSort(next)
    saveSort(SORT_STORAGE, next)
  }

  const kill = async (row: WorkspaceProcess): Promise<void> => {
    if (!(await confirmAction(`Завершить процесс ${row.pid} (${row.cmd})?`))) return
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
      <div className="ws-toolbar">
        <input
          className="search"
          placeholder="Поиск: имя, пользователь, PID, состояние - можно несколько слов"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') setFilter('')
          }}
        />
        {!loading && rows.length > 0 && (
          <span className="hint ws-count">
            {shown.length === rows.length ? rows.length : `${shown.length} из ${rows.length}`}
          </span>
        )}
      </div>
      {error && (
        <div className="sftp-error" onClick={() => setError(null)}>
          {error}
        </div>
      )}
      {note && <div className="agent-hint">{note}</div>}
      {!loading && rows.length >= PROCESS_CAP && (
        <div className="agent-hint">
          Показаны первые {PROCESS_CAP} процессов по загрузке процессора - остальные на сервере есть, но сюда не пришли.
        </div>
      )}
      {loading && <div className="hint" style={{ padding: '10px 12px' }}>Загрузка…</div>}
      {!loading && (
        <div className="ws-table-wrap">
          <table className="ws-table">
            <thead>
              <tr>
                {COLUMNS.map((c) => (
                  <SortHeader key={c.key} as="th" label={c.label} sortKey={c.key} sort={sort} onSort={onSort} />
                ))}
                <th />
              </tr>
            </thead>
            <tbody>
              {shown.length === 0 && (
                <tr>
                  <td colSpan={7} className="hint" style={{ padding: '12px' }}>
                    Ничего не найдено.
                  </td>
                </tr>
              )}
              {shown.map((r) => (
                <tr key={r.pid}>
                  <td className="mono">{r.pid}</td>
                  <td>{r.user}</td>
                  <MetricCell value={r.cpu} />
                  <MetricCell value={r.mem} kind="mem" />
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
