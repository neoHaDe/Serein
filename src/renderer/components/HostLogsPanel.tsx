import { useCallback, useEffect, useMemo, useState } from 'react'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'
import { LogView, buildErrorReport, normalizeLogText } from './dockerLogs'
import { useCtrlWheelZoom } from '../useCtrlWheelZoom'

export function HostLogsPanel({
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
  const [text, setText] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [filter, setFilter] = useState('')
  const [exportBusy, setExportBusy] = useState(false)
  const { zoom, ref: logsZoomRef, reset: resetZoom } = useCtrlWheelZoom('serein.logs.zoom')

  const reload = useCallback(async () => {
    setLoading(true)
    const res = await window.api.workspace.logs(sessionId)
    setLoading(false)
    if (res.ok) {
      setText(normalizeLogText(res.text ?? ''))
      setError(null)
    } else setError(res.error ?? 'Не удалось прочитать логи')
  }, [sessionId])

  useEffect(() => {
    void reload()
  }, [reload])

  const view = useMemo(() => {
    const q = filter.trim().toLowerCase()
    if (!q) return text
    return text
      .split('\n')
      .filter((line) => line.toLowerCase().includes(q))
      .join('\n')
  }, [text, filter])

  const detach = async (): Promise<void> => {
    if (!panelTitle) return
    await openDetachedWorkspace({ tool: 'logs', sessionId, title: panelTitle })
    onDetached?.()
  }

  const exportErrors = async (): Promise<void> => {
    setExportBusy(true)
    const report = buildErrorReport(text, { title: panelTitle ?? 'Логи хоста' })
    const stamp = new Date().toISOString().slice(0, 16).replace(/[:T]/g, '-')
    const name = `serein-errors-${stamp}.txt`
    try {
      const res = await window.api.exportText(report, name)
      if (!res.saved) return
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setExportBusy(false)
    }
  }

  return (
    <div className={'ws-panel' + (fill ? ' fill' : '')}>
      <div className="ws-head">
        <span className="ws-head-title"><Icon name="logs" size={15} /> Логи хоста</span>
        <div style={{ display: 'flex', gap: 6 }}>
          {panelTitle && onDetached && <WsDetachButton onClick={detach} />}
          <button className="mini" title="Масштаб (Ctrl+колесо)" onClick={resetZoom}>
            {Math.round(zoom * 100)}%
          </button>
          <button
            className="mini"
            title="Выгрузить отчёт по ошибкам"
            disabled={exportBusy || loading || !text}
            onClick={() => void exportErrors()}
          >
            <Icon name="import" size={14} />
          </button>
          <button
            className="mini"
            title="Копировать видимое"
            onClick={() => void window.api.clipboard.write(view)}
          >
            <Icon name="copy" size={14} />
          </button>
          <button className="mini" title="Обновить" onClick={() => void reload()}>
            <Icon name="refresh" size={14} />
          </button>
        </div>
      </div>
      <div className="ws-toolbar">
        <input
          className="search"
          placeholder="Фильтр по строке…"
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
        <div className="ws-logs-wrap">
          <LogView text={view} anchorEnd fill zoom={zoom} zoomRef={logsZoomRef} />
        </div>
      )}
    </div>
  )
}
