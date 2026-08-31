import { useEffect, useMemo, useRef, useState } from 'react'
import type { MultiExecResult, ServerConfig } from '../../shared/types'
import { errText } from '../errText'
import { Icon } from './Icon'

const UNGROUPED = ''
const UNGROUPED_TITLE = 'Без группы'

interface Props {
  servers: ServerConfig[]
  onClose: () => void
}

/** Команду выполняем только там, где есть что выполнять. */
function runnable(s: ServerConfig): boolean {
  return s.connection !== 'serial' && s.connection !== 'telnet' && s.connection !== 'raw'
}

function stateLabel(r: MultiExecResult): string {
  if (r.state === 'skipped') return 'пропущен'
  if (r.state === 'failed') return 'не дошли'
  return r.code === 0 ? 'готово' : `код ${r.code}`
}

/**
 * Одна команда на нескольких серверах.
 *
 * Шаг подтверждения здесь не формальность: это единственное место в приложении, где одно
 * нажатие меняет состояние сразу нескольких машин. Поэтому перед запуском показываем
 * и саму команду, и поимённый список хостов — ошибиться выбором проще, чем текстом.
 */
export function MultiExecModal({ servers, onClose }: Props): JSX.Element {
  const [command, setCommand] = useState('')
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [confirming, setConfirming] = useState(false)
  const [running, setRunning] = useState(false)
  const [results, setResults] = useState<MultiExecResult[]>([])
  const [error, setError] = useState('')
  const [open, setOpen] = useState<Set<string>>(new Set())
  const commandRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    commandRef.current?.focus()
  }, [])

  // Результаты приходят по одному, как только хост ответил: на десяти машинах ждать
  // самую медленную, чтобы увидеть первую, незачем.
  useEffect(() => {
    if (!running) return
    const off = window.api.multi.onResult((p) => {
      setResults((prev) => [...prev, p.result])
    })
    return off
  }, [running])

  const groups = useMemo(() => {
    const map = new Map<string, ServerConfig[]>()
    for (const s of servers.filter(runnable)) {
      const g = s.group?.trim() || UNGROUPED
      if (!map.has(g)) map.set(g, [])
      map.get(g)!.push(s)
    }
    for (const list of map.values()) {
      list.sort((a, b) => (a.order ?? 0) - (b.order ?? 0) || a.name.localeCompare(b.name))
    }
    return [...map.entries()].sort(([a], [b]) => (a === UNGROUPED ? 1 : b === UNGROUPED ? -1 : a.localeCompare(b)))
  }, [servers])

  const toggle = (id: string): void =>
    setPicked((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  const toggleGroup = (items: ServerConfig[]): void =>
    setPicked((prev) => {
      const next = new Set(prev)
      const allOn = items.every((s) => next.has(s.id))
      for (const s of items) {
        if (allOn) next.delete(s.id)
        else next.add(s.id)
      }
      return next
    })

  const chosen = servers.filter((s) => picked.has(s.id))

  const stop = (): void => {
    void window.api.multi.cancel()
    setRunning(false)
  }

  const start = async (): Promise<void> => {
    setConfirming(false)
    setRunning(true)
    setResults([])
    setError('')
    try {
      await window.api.multi.exec([...picked], command.trim())
    } catch (e) {
      setError(errText(e))
    } finally {
      setRunning(false)
    }
  }

  const canRun = command.trim().length > 0 && picked.size > 0

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
        <h2>Выполнить на нескольких серверах</h2>

        <label>
          Команда
          <input
            ref={commandRef}
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            placeholder="uptime"
            disabled={running}
          />
        </label>

        <div className="multi-pick">
          {groups.length === 0 && <div className="hint">Нет серверов, на которых можно выполнить команду.</div>}
          {groups.map(([group, items]) => {
            const allOn = items.every((s) => picked.has(s.id))
            const folded = open.has(group)
            return (
              <div key={group || '__ungrouped__'} className="multi-group">
                <div className="multi-group-head">
                  <label className="checkbox-row">
                    <input
                      type="checkbox"
                      checked={allOn}
                      onChange={() => toggleGroup(items)}
                      disabled={running}
                    />
                    {group || UNGROUPED_TITLE}
                    <span className="group-title-count">{items.length}</span>
                  </label>
                  <button
                    className="mini"
                    title={folded ? 'Развернуть' : 'Свернуть'}
                    onClick={() =>
                      setOpen((p) => {
                        const n = new Set(p)
                        if (n.has(group)) n.delete(group)
                        else n.add(group)
                        return n
                      })
                    }
                  >
                    <Icon name={folded ? 'chevron-right' : 'chevron-down'} size={12} />
                  </button>
                </div>
                {!folded &&
                  items.map((s) => (
                    <label key={s.id} className="checkbox-row multi-item">
                      <input
                        type="checkbox"
                        checked={picked.has(s.id)}
                        onChange={() => toggle(s.id)}
                        disabled={running}
                      />
                      {s.name}
                      <span className="multi-host">
                        {s.username}@{s.host}
                      </span>
                    </label>
                  ))}
              </div>
            )
          })}
        </div>

        {error && <div className="settings-msg err">{error}</div>}

        {(running || results.length > 0) && (
          <div className="multi-results">
            <div className="settings-section-title">
              Результаты {results.length}/{picked.size}
            </div>
            {results.map((r) => (
              <details key={r.serverId} className={'multi-result ' + r.state}>
                <summary>
                  <span className="multi-result-name">{r.name}</span>
                  <span className="multi-result-state">{stateLabel(r)}</span>
                  {r.ms !== undefined && <span className="multi-result-ms">{r.ms} мс</span>}
                </summary>
                <pre>{r.error ?? [r.stdout, r.stderr].filter(Boolean).join('\n') ?? ''}</pre>
              </details>
            ))}
          </div>
        )}

        {confirming && (
          <div className="multi-confirm">
            Выполнить <code>{command.trim()}</code> на {chosen.length} серверах?
            <div className="multi-confirm-list">{chosen.map((s) => s.name).join(', ')}</div>
          </div>
        )}

        <div className="modal-actions">
          <button onClick={onClose}>{results.length && !running ? 'Закрыть' : 'Отмена'}</button>
          {running && (
            <button className="danger" onClick={stop}>
              Остановить
            </button>
          )}
          {confirming ? (
            <button className="primary" onClick={() => void start()}>
              Да, выполнить
            </button>
          ) : (
            <button
              className="primary"
              disabled={!canRun || running}
              onClick={() => setConfirming(true)}
            >
              {running ? 'Выполняется…' : `Выполнить на ${picked.size}`}
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
