import { useEffect, useMemo, useState } from 'react'
import { errText } from '../errText'
import {
  actionLabel,
  detailText,
  matchesEntry,
  serverText,
  timeText,
  verifyText,
  type ActionLogEntry,
  type ActionLogStatus,
  type ActionLogVerify
} from '../actionLog'

/** Столько последних записей подгружается в окно. Всё остальное - в выгрузке. */
const LOAD_LIMIT = 2000
/** Столько строк рисуется разом. */
const SHOW_LIMIT = 500

/**
 * Журнал действий: что и где делали из этого приложения.
 *
 * Окно только читает. Писать в журнал, править или удалять записи отсюда нельзя, а
 * проверка целостности показывает, не пропало ли и не изменилось ли что-то на диске.
 */
export function ActionLogModal({ onClose }: { onClose: () => void }): JSX.Element {
  const [entries, setEntries] = useState<ActionLogEntry[]>([])
  const [status, setStatus] = useState<ActionLogStatus | null>(null)
  const [verify, setVerify] = useState<ActionLogVerify | null>(null)
  const [query, setQuery] = useState('')
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)

  const load = async (): Promise<void> => {
    setLoading(true)
    try {
      const [list, st] = await Promise.all([window.api.actionLog.list(LOAD_LIMIT), window.api.actionLog.status()])
      setEntries(list)
      setStatus(st)
    } catch (e) {
      setMsg({ ok: false, text: errText(e) })
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void load()
  }, [])

  const shown = useMemo(() => entries.filter((e) => matchesEntry(e, query)), [entries, query])

  const runVerify = async (): Promise<void> => {
    setBusy(true)
    setMsg(null)
    try {
      setVerify(await window.api.actionLog.verify())
    } catch (e) {
      setMsg({ ok: false, text: errText(e) })
    } finally {
      setBusy(false)
    }
  }

  const runExport = async (): Promise<void> => {
    setBusy(true)
    setMsg(null)
    try {
      const res = await window.api.actionLog.exportAll()
      if (res.saved) setMsg({ ok: true, text: `Выгружено записей: ${res.count} - ${res.path}` })
    } catch (e) {
      setMsg({ ok: false, text: errText(e) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal modal-wide action-log" onClick={(e) => e.stopPropagation()}>
        <h2>Журнал действий</h2>
        {status && (
          <div className="hint">
            {status.enabled ? 'Журнал ведётся' : 'Журнал выключен в настройках'} · {status.dir}
            {status.syslog &&
              ` · syslog ${status.syslog.tcp ? 'TCP' : 'UDP'} ${status.syslog.host}:${status.syslog.port}, отправлено ${status.syslogSent}` +
                (status.syslogFailed > 0 ? `, не ушло ${status.syslogFailed}` : '')}
          </div>
        )}
        <div className="ws-toolbar">
          <input
            className="search"
            placeholder="Поиск: сервер, действие, команда, пользователь - можно несколько слов"
            value={query}
            autoFocus
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setQuery('')
            }}
          />
          {!loading && entries.length > 0 && (
            <span className="hint ws-count">
              {shown.length === entries.length ? entries.length : `${shown.length} из ${entries.length}`}
            </span>
          )}
        </div>
        {verify && <div className={'settings-msg' + (verify.ok ? ' ok' : ' err')}>{verifyText(verify)}</div>}
        {msg && <div className={'settings-msg' + (msg.ok ? ' ok' : ' err')}>{msg.text}</div>}
        {loading && <div className="hint">Загрузка…</div>}
        {!loading && (
          <div className="ws-table-wrap action-log-table">
            <table className="ws-table">
              <thead>
                <tr>
                  <th>Время</th>
                  <th>Кто</th>
                  <th>Сервер</th>
                  <th>Действие</th>
                  <th>Подробности</th>
                </tr>
              </thead>
              <tbody>
                {shown.length === 0 && (
                  <tr>
                    <td colSpan={5} className="hint" style={{ padding: 12 }}>
                      {entries.length === 0 ? 'Записей пока нет.' : 'Ничего не найдено.'}
                    </td>
                  </tr>
                )}
                {shown.slice(0, SHOW_LIMIT).map((e) => (
                  <tr key={e.seq} className={e.ok ? '' : 'action-log-failed'}>
                    <td className="mono" title={`#${e.seq} · ${e.t}`}>
                      {timeText(e.t)}
                    </td>
                    <td title={e.actor?.machine ?? ''}>{e.actor?.user}</td>
                    <td>{serverText(e)}</td>
                    <td>{actionLabel(e.action)}</td>
                    <td className="ws-cmd" title={detailText(e) + (e.error ? `\nОшибка: ${e.error}` : '')}>
                      {detailText(e)}
                      {e.error && <span className="action-log-error"> - {e.error}</span>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {shown.length > SHOW_LIMIT && (
          <div className="hint">
            Показаны последние {SHOW_LIMIT} из {shown.length}. Уточните поиск или выгрузите журнал целиком.
          </div>
        )}
        <div className="modal-actions">
          <button onClick={onClose}>Закрыть</button>
          <button onClick={() => void load()} disabled={loading || busy}>
            Обновить
          </button>
          <button onClick={() => void runVerify()} disabled={busy}>
            Проверить целостность
          </button>
          <button className="primary" onClick={() => void runExport()} disabled={busy}>
            Выгрузить весь журнал
          </button>
        </div>
      </div>
    </div>
  )
}
