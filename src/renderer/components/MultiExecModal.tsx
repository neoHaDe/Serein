import { useEffect, useMemo, useRef, useState } from 'react'
import type { MultiExecResult, ServerConfig } from '../../shared/types'
import { errText } from '../errText'
import { useSettings } from '../SettingsContext'
import {
  CONCURRENCY_PRESETS,
  DEFAULT_CONCURRENCY,
  DEFAULT_TIMEOUT_SEC,
  MAX_CONCURRENCY,
  MAX_TIMEOUT_SEC,
  clampConcurrency,
  clampTimeout,
  fleetReport,
  mergeResults,
  retryIds,
  stateLabel,
  summarizeFleet,
  summaryText
} from '../fleetReport'
import { Icon } from './Icon'
import { confirmAction } from '../confirmDialog'

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

/**
 * Одна команда на нескольких серверах.
 *
 * Шаг подтверждения здесь не формальность: это единственное место в приложении, где одно
 * нажатие меняет состояние сразу нескольких машин. Поэтому перед запуском показываем
 * и саму команду, и поимённый список хостов - ошибиться выбором проще, чем текстом.
 */
export function MultiExecModal({ servers, onClose }: Props): JSX.Element {
  const { settings, update } = useSettings()
  const concurrency = clampConcurrency(settings.fleetConcurrency ?? DEFAULT_CONCURRENCY)
  const timeoutSec = clampTimeout(settings.fleetTimeoutSec ?? DEFAULT_TIMEOUT_SEC)
  const [customConcurrency, setCustomConcurrency] = useState(!CONCURRENCY_PRESETS.includes(concurrency))

  const [command, setCommand] = useState('')
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [confirming, setConfirming] = useState(false)
  const [running, setRunning] = useState(false)
  const [results, setResults] = useState<MultiExecResult[]>([])
  // Команда и время прогона, к которому относятся результаты. Поле ввода могли поменять
  // после запуска - повтор и отчёт обязаны взять то, что выполнялось на самом деле.
  const [ranCommand, setRanCommand] = useState('')
  const [ranAt, setRanAt] = useState<Date | null>(null)
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null)
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
      setResults((prev) => mergeResults(prev, [p.result]))
      setProgress({ done: p.done, total: p.total })
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
  const toRetry = retryIds(results)

  const stop = (): void => {
    // Бэкенд не начинает новых подключений после остановки: оставшиеся хосты придут
    // пропущенными, поэтому окно ждёт их, а не бросает прогон на полуслове.
    void window.api.multi.cancel()
  }

  const run = async (ids: string[], cmd: string): Promise<void> => {
    setRunning(true)
    setError('')
    setProgress({ done: 0, total: ids.length })
    try {
      await window.api.multi.exec(ids, cmd, { concurrency, timeoutSec })
    } catch (e) {
      setError(errText(e))
    } finally {
      setRunning(false)
    }
  }

  const start = (): void => {
    setConfirming(false)
    setResults([])
    setRanCommand(command.trim())
    setRanAt(new Date())
    void run([...picked], command.trim())
  }

  const retry = async (): Promise<void> => {
    if (!(await confirmAction(`Повторить «${ranCommand}» на ${toRetry.length} серверах, где команда не удалась?`))) return
    void run(toRetry, ranCommand)
  }

  const saveReport = async (): Promise<void> => {
    try {
      await window.api.exportText(fleetReport(ranCommand, results, ranAt ?? new Date()), 'fleet-report.txt')
    } catch (e) {
      setError(errText(e))
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

        <div className="multi-options">
          <label>
            Одновременно
            <select
              value={customConcurrency ? 'custom' : String(concurrency)}
              disabled={running}
              onChange={(e) => {
                if (e.target.value === 'custom') {
                  setCustomConcurrency(true)
                  return
                }
                setCustomConcurrency(false)
                update({ fleetConcurrency: clampConcurrency(Number(e.target.value)) })
              }}
            >
              {CONCURRENCY_PRESETS.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
              <option value="custom">своё число</option>
            </select>
          </label>
          {customConcurrency && (
            <label>
              Хостов
              <input
                type="number"
                min={1}
                max={MAX_CONCURRENCY}
                value={concurrency}
                disabled={running}
                onChange={(e) => update({ fleetConcurrency: clampConcurrency(Number(e.target.value)) })}
              />
            </label>
          )}
          <label>
            Ждать хост, с
            <input
              type="number"
              min={1}
              max={MAX_TIMEOUT_SEC}
              value={timeoutSec}
              disabled={running}
              onChange={(e) => update({ fleetTimeoutSec: clampTimeout(Number(e.target.value)) })}
            />
          </label>
        </div>

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
            <div className="multi-results-head">
              <div className="settings-section-title">
                {running && progress ? `Выполняется ${progress.done}/${progress.total}` : 'Результаты'}
              </div>
              {!running && results.length > 0 && (
                <div className="multi-results-actions">
                  {toRetry.length > 0 && (
                    <button className="mini" onClick={retry}>
                      Повторить упавшие ({toRetry.length})
                    </button>
                  )}
                  <button className="mini" onClick={() => void saveReport()}>
                    Сохранить отчёт
                  </button>
                </div>
              )}
            </div>
            {results.length > 0 && <div className="multi-summary">{summaryText(summarizeFleet(results))}</div>}
            {results.map((r) => (
              <details key={r.serverId} className={'multi-result ' + r.state}>
                <summary>
                  <span className="multi-result-name">{r.name}</span>
                  <span className="multi-result-state">{stateLabel(r)}</span>
                  {r.ms !== undefined && <span className="multi-result-ms">{r.ms} мс</span>}
                </summary>
                <pre>{r.error ?? [r.stdout, r.stderr].filter(Boolean).join('\n')}</pre>
              </details>
            ))}
          </div>
        )}

        {confirming && (
          <div className="multi-confirm">
            Выполнить <code>{command.trim()}</code> на {chosen.length} серверах, по {concurrency} одновременно?
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
            <button className="primary" onClick={start}>
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
