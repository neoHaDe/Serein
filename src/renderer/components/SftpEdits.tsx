import { useEffect, useState } from 'react'
import type { RemoteEditStatus } from '../../shared/types'

/** Файлы этой сессии, которые сейчас правятся во внешнем редакторе, и их состояние. */
export function useRemoteEdits(sessionId: string): RemoteEditStatus[] {
  const [edits, setEdits] = useState<Record<string, RemoteEditStatus>>({})
  useEffect(() => {
    return window.api.sftp.onEditStatus((s) => {
      // Как и у передач: только своя сессия, иначе панель одного сервера показывала бы
      // правки файлов другого.
      if (s.sessionId !== sessionId) return
      setEdits((prev) => {
        const next = { ...prev }
        if (s.state === 'stopped') delete next[s.remotePath]
        else next[s.remotePath] = s
        return next
      })
    })
  }, [sessionId])
  return Object.values(edits)
}

function stateIcon(s: RemoteEditStatus['state']): string {
  if (s === 'uploading') return '⬆'
  if (s === 'synced') return '✓'
  if (s === 'error' || s === 'conflict') return '⚠'
  return '✎'
}

function stateLabel(s: RemoteEditStatus['state']): string {
  if (s === 'uploading') return 'заливка…'
  if (s === 'synced') return 'сохранено'
  if (s === 'error') return 'ошибка'
  if (s === 'conflict') return 'изменён на сервере'
  return 'редактируется'
}

/** Строки «редактируется / сохранено / конфликт» над списком файлов. */
export function SftpEditList({ sessionId, edits }: { sessionId: string; edits: RemoteEditStatus[] }): JSX.Element | null {
  if (edits.length === 0) return null
  return (
    <div className="sftp-edits">
      {edits.map((ed) => (
        <div key={ed.remotePath} className="sftp-edit-row">
          <span className="edit-state" title={ed.error}>
            {stateIcon(ed.state)}
          </span>
          <span className="sftp-name">{ed.remotePath.split('/').pop()}</span>
          <span className="edit-label">{stateLabel(ed.state)}</span>
          <button
            className="mini"
            title="Перестать следить"
            onClick={() => window.api.sftp.editStop(sessionId, ed.remotePath)}
          >
            ✕
          </button>
          {/* Текст ошибки - на виду, а не только в подсказке: при конфликте в нём путь к
              сохранённой правке, и искать его под значком никто не станет. */}
          {(ed.state === 'error' || ed.state === 'conflict') && ed.error && (
            <div className="sftp-edit-error">{ed.error}</div>
          )}
        </div>
      ))}
    </div>
  )
}
