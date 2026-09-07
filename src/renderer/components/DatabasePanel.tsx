import { useCallback, useEffect, useRef, useState } from 'react'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'
import { errText } from '../errText'
import { cellText, isNull, needsConfirm, summarize, type QueryResult } from '../dbQuery'
import { forget, isGone, recall, remember, update } from '../dbMemory'

/**
 * Базы данных рядом с сервером.
 *
 * Соединение идёт каналом внутри уже открытой SSH-сессии, поэтому здесь не спрашивают
 * адрес «снаружи»: база слушает петлю сервера, и по умолчанию мы туда и целимся. Правила
 * показа результата и предупреждений живут в `dbQuery.ts` - там же тесты.
 *
 * Уход на другую вкладку соединение **не рвёт**: оно привязано к SSH-сессии, а не к тому,
 * открыта ли панель. Что показать при возвращении, помнит `dbMemory.ts` - но только внутри
 * своего окна. Откреплённая панель живёт в отдельном веб-контексте, и там эта память пуста,
 * поэтому при появлении на пустом месте панель переспрашивает приложение.
 */

type Kind = 'postgres' | 'mysql' | 'redis'

interface Props {
  sessionId: string
  panelTitle?: string
  onDetached?: () => void
  fill?: boolean
}

/** Подсказки в поле запроса: у SQL и у Redis разный язык, и пустой экран бесполезен. */
const HINT: Record<Kind, string> = {
  postgres: 'SELECT * FROM pg_stat_activity LIMIT 20;',
  mysql: 'SHOW FULL PROCESSLIST;',
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
  mysql: [
    { label: 'Таблицы', text: "SELECT table_schema, table_name FROM information_schema.tables WHERE table_schema NOT IN ('mysql','information_schema','performance_schema','sys') ORDER BY 1, 2" },
    { label: 'Размеры баз', text: 'SELECT table_schema AS база, ROUND(SUM(data_length + index_length) / 1024 / 1024) AS мегабайт FROM information_schema.tables GROUP BY table_schema ORDER BY 2 DESC' },
    { label: 'Активные запросы', text: 'SHOW FULL PROCESSLIST' },
    { label: 'Версия', text: 'SELECT VERSION() AS версия' }
  ],
  redis: [
    { label: 'Сервер', text: 'INFO server' },
    { label: 'Память', text: 'INFO memory' },
    { label: 'Ключей в базе', text: 'DBSIZE' },
    { label: 'Клиенты', text: 'CLIENT LIST' }
  ]
}

/** Кого подставлять в поле пользователя. У Redis имени обычно нет вовсе. */
const DEFAULT_USER: Record<Kind, string> = { postgres: 'postgres', mysql: 'root', redis: '' }

/** Подсказки в пустых полях - то же, что подставит бэкенд, если оставить их пустыми. */
const DEFAULT_PORT: Record<Kind, number> = { postgres: 5432, mysql: 3306, redis: 6379 }
const DEFAULT_DB: Record<Kind, string> = { postgres: 'postgres', mysql: 'mysql', redis: '0' }

export function DatabasePanel({ sessionId, panelTitle, onDetached, fill }: Props): JSX.Element {
  // Что было открыто в прошлый раз на этой же сессии. Читаем один раз при создании
  // панели: дальше состояние живёт в React, а сюда только записывается.
  const saved = useRef(recall(sessionId)).current
  const idRef = useRef<string | null>(saved?.connectionId ?? null)

  const [kind, setKind] = useState<Kind>((saved?.form.kind as Kind) ?? 'postgres')
  const [host, setHost] = useState(saved?.form.host ?? '127.0.0.1')
  const [port, setPort] = useState(saved?.form.port ?? '')
  const [user, setUser] = useState(saved?.form.user ?? 'postgres')
  // Пароль намеренно не восстанавливаем: пока соединение живо, он не нужен, а держать
  // его в памяти дольше формы - плата без выгоды.
  const [password, setPassword] = useState('')
  const [database, setDatabase] = useState(saved?.form.database ?? '')

  const [connected, setConnected] = useState<{ kind: string; host: string; port: number } | null>(
    saved?.info ?? null
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const [text, setText] = useState(saved?.text ?? '')
  const [result, setResult] = useState<QueryResult | null>(saved?.result ?? null)

  /**
   * Подхватывает соединение, о котором панель не знает.
   *
   * Это случай откреплённого окна: память модуля там своя и пустая, а база открыта и
   * живёт в приложении. Без этого отделение панели выглядело бы как обрыв связи, хотя
   * рвать было нечего - и человек полез бы вводить пароль заново.
   */
  useEffect(() => {
    if (saved) return
    let ушли = false
    void window.api.db.current(sessionId).then((live) => {
      if (ушли || !live) return
      idRef.current = live.id
      const shown = { kind: live.kind, host: live.host, port: live.port }
      setConnected(shown)
      setHost(live.host)
      setPort(String(live.port))
      setKind(live.kind as Kind)
      remember(sessionId, {
        connectionId: live.id,
        info: shown,
        form: { kind: live.kind, host: live.host, port: String(live.port), user: '', database: '' },
        text: '',
        result: null
      })
    })
    return () => {
      ушли = true
    }
  }, [saved, sessionId])

  const disconnect = useCallback(() => {
    const id = idRef.current
    idRef.current = null
    if (id) void window.api.db.close(id)
    forget(sessionId)
    setConnected(null)
    setResult(null)
  }, [sessionId])

  /**
   * Соединение больше не живо - но не по нашей воле.
   *
   * Так бывает, когда SSH-сессия закрылась, пока панель была на другой вкладке: канал
   * жил внутри неё. Держаться за такое соединение значит показывать таблицу, за которой
   * ничего нет, поэтому возвращаемся к форме и говорим почему.
   */
  const connectionGone = useCallback(() => {
    idRef.current = null
    forget(sessionId)
    setConnected(null)
    setResult(null)
    setError('Соединение с базой закрылось вместе с сессией - подключитесь заново')
  }, [sessionId])

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
      const shown = { kind: info.kind, host: info.host, port: info.port }
      setConnected(shown)
      remember(sessionId, {
        connectionId: info.id,
        info: shown,
        form: { kind, host, port, user, database },
        text,
        result: null
      })
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
      // Помним именно содержимое поля, а не выполненный запрос: заготовки из списка
      // текст в поле не меняют, и подменять его при возвращении было бы неожиданно.
      update(sessionId, { result: out })
    } catch (e) {
      const msg = errText(e)
      if (isGone(msg)) {
        connectionGone()
        return
      }
      setError(msg)
      setResult(null)
      update(sessionId, { result: null })
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
    // Ctrl+Enter - выполнить: перевод строки в запросе нужен чаще, чем запуск по Enter.
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
                  setUser(DEFAULT_USER[k])
                }}
              >
                <option value="postgres">PostgreSQL</option>
                <option value="mysql">MySQL / MariaDB</option>
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
                placeholder={String(DEFAULT_PORT[kind])}
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
              {kind === 'redis' ? 'Номер базы' : 'База'}
              <input
                value={database}
                onChange={(e) => setDatabase(e.target.value)}
                placeholder={DEFAULT_DB[kind]}
              />
            </label>
          </div>
          <div className="agent-hint">
            Подключение идёт внутри этой SSH-сессии: порт наружу открывать не нужно, адрес -
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
              onChange={(e) => {
                setText(e.target.value)
                update(sessionId, { text: e.target.value })
              }}
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
