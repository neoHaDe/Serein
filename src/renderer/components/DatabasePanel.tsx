import { useCallback, useEffect, useRef, useState } from 'react'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'
import { errText } from '../errText'
import { cellText, isNull, needsConfirm, summarize, type QueryResult } from '../dbQuery'

/**
 * Базы данных рядом с сервером.
 *
 * Соединение идёт каналом внутри уже открытой SSH-сессии, поэтому здесь не спрашивают
 * адрес «снаружи»: база слушает петлю сервера, и по умолчанию мы туда и целимся. Правила
 * показа результата и предупреждений живут в `dbQuery.ts` — там же тесты.
 */

type Kind = 'postgres' | 'redis'

interface Props {
  sessionId: string
  panelTitle?: string
  onDetached?: () => void
  fill?: boolean
}

/** Подсказки в поле запроса: у SQL и у Redis разный язык, и пустой экран бесполезен. */
const HINT: Record<Kind, string> = {
  postgres: 'SELECT * FROM pg_stat_activity LIMIT 20;',
  redis: 'INFO server'
}

/** Что показать в списке, пока пользователь не написал свой запрос. */
const STARTERS: Record<Kind, { label: string; text: string }[]> = {
  postgres: [
    { label: 'Таблицы', text: "SELECT table_schema, table_name FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY 1, 2" },
    { label: 'Размеры баз', text: 'SELECT datname, pg_size_pretty(pg_database_size(datname)) AS размер FROM pg_database ORDER BY pg_database_size(datname) DESC' },
    { label: 'Активные запросы', text: "SELECT pid, usename, state, query FROM pg_stat_activity WHERE state <> 'idle'" },
    { label: 'Версия', text: 'SELECT version()' }
  ],
  redis: [
    { label: 'Сервер', text: 'INFO server' },
    { label: 'Память', text: 'INFO memory' },
    { label: 'Ключей в базе', text: 'DBSIZE' },
    { label: 'Клиенты', text: 'CLIENT LIST' }
  ]
}

export function DatabasePanel({ sessionId, panelTitle, onDetached, fill }: Props): JSX.Element {
  const idRef = useRef<string | null>(null)

  const [kind, setKind] = useState<Kind>('postgres')
  const [host, setHost] = useState('127.0.0.1')
  const [port, setPort] = useState('')
  const [user, setUser] = useState('postgres')
  const [password, setPassword] = useState('')
  const [database, setDatabase] = useState('')

  const [connected, setConnected] = useState<{ kind: string; host: string; port: number } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const [text, setText] = useState('')
  const [result, setResult] = useState<QueryResult | null>(null)

  const disconnect = useCallback(() => {
    const id = idRef.current
    idRef.current = null
    if (id) void window.api.db.close(id)
    setConnected(null)
    setResult(null)
  }, [])

  // Сессия закрылась — соединение с базой шло внутри неё и тоже больше не живо.
  useEffect(() => () => disconnect(), [disconnect])

  const connect = async (): Promise<void> => {
    setBusy(true)
    setError('')
    try {
      const info = await window.api.db.open(sessionId, {
        kind,
        host,
        port: port ? Number(port) : undefined,
        user,
        password,
        database
      })
      idRef.current = info.id
      setConnected({ kind: info.kind, host: info.host, port: info.port })
    } catch (e) {
      setError(errText(e))
    } finally {
      setBusy(false)
    }
  }

  const run = async (sql?: string): Promise<void> => {
    const id = idRef.current
    const query = (sql ?? text).trim()
    if (!id || !query) return

    // Необратимое действие подтверждаем до выполнения, а не сообщаем после.
    const warn = needsConfirm(query)
    if (warn && !confirm(`${warn}\n\nВыполнить?`)) return

    setBusy(true)
    setError('')
    try {
      const out = await window.api.db.query(id, query)
      setResult(out)
    } catch (e) {
      setError(errText(e))
      setResult(null)
    } finally {
      setBusy(false)
    }
  }

  const detach = async (): Promise<void> => {
    if (!panelTitle) return
    await openDetachedWorkspace({ tool: 'databases', sessionId, title: panelTitle })
    onDetached?.()
  }

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>): void => {
    // Ctrl+Enter — выполнить: перевод строки в запросе нужен чаще, чем запуск по Enter.
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault()
      void run()
    }
  }

  return (
    <div className={'ws-panel db-panel' + (fill ? ' fill' : '')}>
      <div className="ws-head">
        <span className="ws-head-title">
          <Icon name="list" size={15} /> Базы данных
          {connected && (
            <span className="db-badge">
              {connected.kind} · {connected.host}:{connected.port}
            </span>
          )}
        </span>
        <div style={{ display: 'flex', gap: 6 }}>
          {panelTitle && onDetached && <WsDetachButton onClick={detach} />}
          {connected && (
            <button className="mini" title="Отключиться" onClick={disconnect}>
              <Icon name="close" size={14} />
            </button>
          )}
        </div>
      </div>

      {!connected && (
        <div className="db-connect">
          <div className="db-form">
            <label>
              База
              <select
                value={kind}
                onChange={(e) => {
                  const k = e.target.value as Kind
                  setKind(k)
                  // Пользователь по умолчанию свой у каждой базы, а у Redis его обычно нет.
                  setUser(k === 'postgres' ? 'postgres' : '')
                }}
              >
                <option value="postgres">PostgreSQL</option>
                <option value="redis">Redis</option>
              </select>
            </label>
            <label>
              Адрес на сервере
              <input value={host} onChange={(e) => setHost(e.target.value)} placeholder="127.0.0.1" />
            </label>
            <label>
              Порт
              <input
                value={port}
                onChange={(e) => setPort(e.target.value.replace(/\D/g, ''))}
                placeholder={kind === 'postgres' ? '5432' : '6379'}
              />
            </label>
            <label>
              Пользователь
              <input value={user} onChange={(e) => setUser(e.target.value)} />
            </label>
            <label>
              Пароль
              <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} />
            </label>
            <label>
              {kind === 'postgres' ? 'База' : 'Номер базы'}
              <input
                value={database}
                onChange={(e) => setDatabase(e.target.value)}
                placeholder={kind === 'postgres' ? 'postgres' : '0'}
              />
            </label>
          </div>
          <div className="agent-hint">
            Подключение идёт внутри этой SSH-сессии: порт наружу открывать не нужно, адрес —
            такой, каким его видит сам сервер.
          </div>
          {error && <div className="db-error">{error}</div>}
          <button className="primary" disabled={busy} onClick={() => void connect()}>
            {busy ? 'Подключение…' : 'Подключиться'}
          </button>
        </div>
      )}

      {connected && (
        <>
          <div className="db-starters">
            {STARTERS[kind].map((s) => (
              <button key={s.label} className="chip" onClick={() => void run(s.text)}>
                {s.label}
              </button>
            ))}
          </div>

          <div className="db-editor">
            <textarea
              value={text}
              placeholder={HINT[kind]}
              spellCheck={false}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={onKeyDown}
            />
            <button className="primary" disabled={busy} onClick={() => void run()}>
              {busy ? 'Выполняется…' : 'Выполнить (Ctrl+Enter)'}
            </button>
          </div>

          {error && <div className="db-error">{error}</div>}

          {result && (
            <div className="db-result">
              <div className="ws-table-wrap">
                <table className="ws-table">
                  <thead>
                    <tr>
                      {result.columns.map((c) => (
                        <th key={c}>{c}</th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {result.rows.map((row, i) => (
                      <tr key={i}>
                        {result.columns.map((c) => (
                          <td key={c} className={isNull(row[c]) ? 'db-null' : undefined}>
                            {cellText(row[c])}
                          </td>
                        ))}
                      </tr>
                    ))}
                    {result.rows.length === 0 && (
                      <tr>
                        <td colSpan={Math.max(1, result.columns.length)} className="hint">
                          Строк нет
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
              <div className="db-summary">{summarize(result)}</div>
            </div>
          )}
        </>
      )}
    </div>
  )
}
