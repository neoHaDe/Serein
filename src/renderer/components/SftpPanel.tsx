import { useCallback, useEffect, useRef, useState } from 'react'
import type { SftpEntry, LocalEntry, TransferItem, RemoteEditStatus } from '../../shared/types'
import { isTextFile } from '../editorLang'
import { Icon } from './Icon'
import { openAuxWindow, sanitizeWindowLabel } from '../auxWindows'
import { useCtrlWheelZoom } from '../useCtrlWheelZoom'
import { AuxDrag, WindowSysButtons } from './WindowChrome'

/** Разбивает абсолютный remote-путь на сегменты-крошки: [{label, path}]. */
function remoteCrumbs(path: string): { label: string; path: string }[] {
  if (!path.startsWith('/')) return [] // относительный путь (напр. '.') — крошки не строим
  const parts = path.split('/').filter(Boolean)
  const crumbs = [{ label: '/', path: '/' }]
  let acc = ''
  for (const p of parts) {
    acc += '/' + p
    crumbs.push({ label: p, path: acc })
  }
  return crumbs
}

interface Props {
  sessionId: string
  onClose: () => void
  width: number
  closing: boolean
  detached?: boolean
  onOpenInEditor?: (remotePath: string) => void
}

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`
  return `${(n / 1024 ** 3).toFixed(2)} GB`
}

function fmtSpeed(bps: number | undefined): string {
  if (!bps || !isFinite(bps) || bps < 256) return ''
  return fmtSize(bps) + '/s'
}

function fmtEta(size: number, transferred: number, bps: number | undefined): string {
  if (!bps || bps < 1024 || !size || transferred >= size) return ''
  const sec = Math.round((size - transferred) / bps)
  if (sec < 1) return ''
  if (sec < 60) return `${sec}с`
  return `${Math.floor(sec / 60)}м ${sec % 60}с`
}

function parentOfRemote(path: string): string {
  if (path === '/' || path === '') return '/'
  const trimmed = path.replace(/\/+$/, '')
  const idx = trimmed.lastIndexOf('/')
  return idx <= 0 ? '/' : trimmed.slice(0, idx)
}

function joinRemote(dir: string, name: string): string {
  return dir.endsWith('/') ? dir + name : dir + '/' + name
}

function joinLocal(dir: string, name: string): string {
  if (dir.endsWith('/') || dir.endsWith('\\')) return dir + name
  const sep = dir.includes('\\') ? '\\' : '/'
  return dir + sep + name
}

export function SftpPanel({ sessionId, onClose, width, closing, detached, onOpenInEditor }: Props): JSX.Element {
  const [path, setPath] = useState('.')
  const [entries, setEntries] = useState<SftpEntry[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [dragOver, setDragOver] = useState(false)

  const [dualPane, setDualPane] = useState(false)
  const [localPath, setLocalPath] = useState('')
  const [localEntries, setLocalEntries] = useState<LocalEntry[]>([])
  const [localDragOver, setLocalDragOver] = useState(false)

  const [transfers, setTransfers] = useState<TransferItem[]>([])
  const rateRef = useRef(new Map<string, { t: number; b: number; bps: number }>())
  const [edits, setEdits] = useState<Record<string, RemoteEditStatus>>({})

  // Инлайн-переименование: имя редактируемой записи + текущее значение поля.
  const [renaming, setRenaming] = useState<string | null>(null)
  const [renameValue, setRenameValue] = useState('')
  const { zoom, ref: zoomRef, reset } = useCtrlWheelZoom('serein.sftp.zoom')

  const pathRef = useRef(path)
  pathRef.current = path
  const localPathRef = useRef(localPath)
  localPathRef.current = localPath

  const load = useCallback(
    async (target: string) => {
      setLoading(true)
      setError(null)
      try {
        const res = await window.api.sftp.list(sessionId, target)
        setPath(res.path)
        setEntries(res.entries)
      } catch (e) {
        setError((e as Error).message)
      } finally {
        setLoading(false)
      }
    },
    [sessionId]
  )

  const loadLocal = useCallback(async (target: string) => {
    try {
      const res = await window.api.localfs.list(target)
      setLocalPath(res.path)
      setLocalEntries(res.entries)
    } catch (e) {
      setError((e as Error).message)
    }
  }, [])

  useEffect(() => {
    load('.')
  }, [load])

  // Подписка на очередь передач: апдейтим элементы по id, по завершении — обновляем списки.
  useEffect(() => {
    const off = window.api.sftp.onTransfer((item) => {
      const now = Date.now()
      const rates = rateRef.current
      let speedBps = 0
      if (item.state === 'active') {
        const prev = rates.get(item.id)
        if (prev && item.transferred > prev.b) {
          const dt = (now - prev.t) / 1000
          if (dt >= 0.2) {
            const inst = (item.transferred - prev.b) / dt
            speedBps = prev.bps > 0 ? prev.bps * 0.55 + inst * 0.45 : inst
            rates.set(item.id, { t: now, b: item.transferred, bps: speedBps })
          } else {
            speedBps = prev.bps
          }
        } else if (!prev) {
          rates.set(item.id, { t: now, b: item.transferred, bps: 0 })
        } else {
          speedBps = prev.bps
        }
      } else {
        rates.delete(item.id)
      }
      const nextItem = { ...item, speedBps }
      setTransfers((prev) => {
        const idx = prev.findIndex((t) => t.id === nextItem.id)
        if (idx === -1) return [...prev, nextItem]
        const next = [...prev]
        next[idx] = nextItem
        return next
      })
      if (item.state === 'done' || item.state === 'error') {
        load(pathRef.current)
        if (localPathRef.current) loadLocal(localPathRef.current)
      } else if (item.state === 'canceled' && localPathRef.current) {
        loadLocal(localPathRef.current)
      }
    })
    const offEdit = window.api.sftp.onEditStatus((s) => {
      setEdits((prev) => ({ ...prev, [s.remotePath]: s }))
      if (s.state === 'stopped') {
        setEdits((prev) => {
          const n = { ...prev }
          delete n[s.remotePath]
          return n
        })
      }
    })
    return () => {
      off()
      offEdit()
    }
  }, [load, loadLocal])

  const toggleDual = (): void => {
    const next = !dualPane
    setDualPane(next)
    if (next && !localPath) void window.api.localfs.home().then((h) => loadLocal(h))
  }

  // ---- Remote actions ----
  const uploadDialog = async (): Promise<void> => {
    try {
      await window.api.sftp.upload(sessionId, path)
    } catch (e) {
      setError((e as Error).message)
    }
  }
  const uploadFolderDialog = async (): Promise<void> => {
    try {
      await window.api.sftp.uploadFolder(sessionId, path)
    } catch (e) {
      setError((e as Error).message)
    }
  }

  const download = async (entry: SftpEntry): Promise<void> => {
    try {
      await window.api.sftp.download(sessionId, joinRemote(path, entry.name))
    } catch (e) {
      setError((e as Error).message)
    }
  }

  const editRemote = async (entry: SftpEntry): Promise<void> => {
    try {
      await window.api.sftp.edit(sessionId, joinRemote(path, entry.name))
    } catch (e) {
      setError((e as Error).message)
    }
  }

  const mkdir = async (): Promise<void> => {
    const name = prompt('Имя новой папки:')
    if (!name) return
    try {
      await window.api.sftp.mkdir(sessionId, joinRemote(path, name))
      load(path)
    } catch (e) {
      setError((e as Error).message)
    }
  }

  const remove = async (entry: SftpEntry): Promise<void> => {
    if (!confirm(`Удалить «${entry.name}»?`)) return
    try {
      await window.api.sftp.remove(sessionId, joinRemote(path, entry.name), entry.type === 'dir')
      load(path)
    } catch (e) {
      setError((e as Error).message)
    }
  }

  const startRename = (entry: SftpEntry): void => {
    setRenaming(entry.name)
    setRenameValue(entry.name)
  }

  const commitRename = async (oldName: string): Promise<void> => {
    const next = renameValue.trim()
    setRenaming(null)
    if (!next || next === oldName) return
    try {
      await window.api.sftp.rename(sessionId, joinRemote(path, oldName), joinRemote(path, next))
      load(path)
    } catch (e) {
      setError((e as Error).message)
    }
  }

  // ---- Drag & drop на удалённую панель (загрузка) ----
  const onRemoteDrop = async (e: React.DragEvent): Promise<void> => {
    e.preventDefault()
    setDragOver(false)
    // 1) Внутреннее перетаскивание из локальной панели.
    const localSrc = e.dataTransfer.getData('x-local-path')
    if (localSrc) {
      await window.api.sftp.uploadPaths(sessionId, path, [localSrc])
      return
    }
    // 2) Файлы из ОС (проводник/рабочий стол).
    const files = Array.from(e.dataTransfer.files)
    if (files.length) {
      const paths = files.map((f) => window.api.files.pathForFile(f)).filter(Boolean)
      if (paths.length) await window.api.sftp.uploadPaths(sessionId, path, paths)
    }
  }

  // ---- Local actions ----
  const uploadLocalEntry = async (entry: LocalEntry): Promise<void> => {
    await window.api.sftp.uploadPaths(sessionId, path, [joinLocal(localPath, entry.name)])
  }
  const downloadToLocal = async (entry: SftpEntry): Promise<void> => {
    if (!localPath) return
    await window.api.sftp.downloadTo(sessionId, joinRemote(path, entry.name), localPath)
  }

  const onLocalDrop = async (e: React.DragEvent): Promise<void> => {
    e.preventDefault()
    setLocalDragOver(false)
    const remoteSrc = e.dataTransfer.getData('x-remote-path')
    if (remoteSrc && localPath) await window.api.sftp.downloadTo(sessionId, remoteSrc, localPath)
  }

  // ---- Transfers ----
  const activeTransfers = transfers.filter(
    (t) => t.state === 'queued' || t.state === 'active' || t.state === 'paused'
  )
  const cancelTransfer = (id: string): void => {
    setTransfers((prev) =>
      prev.map((t) =>
        t.id === id && (t.state === 'queued' || t.state === 'active' || t.state === 'paused')
          ? { ...t, state: 'canceled' as const }
          : t
      )
    )
    void window.api.sftp.cancelTransfer(id)
  }
  const pauseTransfer = (id: string): void => {
    setTransfers((prev) =>
      prev.map((t) => (t.id === id && t.state === 'active' ? { ...t, state: 'paused' as const, speedBps: 0 } : t))
    )
    void window.api.sftp.pauseTransfer(id)
  }
  const resumeTransfer = (id: string): void => {
    setTransfers((prev) =>
      prev.map((t) => (t.id === id && t.state === 'paused' ? { ...t, state: 'active' as const } : t))
    )
    void window.api.sftp.resumeTransfer(id)
  }
  const retryTransfer = (t: TransferItem): void => {
    const parent = (p: string): string => {
      const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'))
      return i <= 0 ? p : p.slice(0, i)
    }
    setTransfers((prev) => prev.filter((x) => x.id !== t.id))
    if (t.direction === 'download') {
      void window.api.sftp.downloadTo(sessionId, t.remotePath, parent(t.localPath))
    } else {
      void window.api.sftp.uploadPaths(sessionId, parent(t.remotePath), [t.localPath])
    }
  }
  const clearFinished = (): void => {
    setTransfers((prev) =>
      prev.filter((t) => t.state === 'queued' || t.state === 'active' || t.state === 'paused')
    )
  }

  const editList = Object.values(edits)

  const detach = async (): Promise<void> => {
    await openAuxWindow({
      label: 'sftp-' + sanitizeWindowLabel(sessionId),
      query: { sftp: '1', sessionId },
      title: 'SFTP',
      width: Math.max(width, 480),
      height: 720
    })
    onClose()
  }

  return (
    <div
      className={'sftp-panel' + (closing ? ' closing' : '') + (detached ? ' detached' : '')}
      ref={zoomRef}
      style={detached ? { zoom } : { width, zoom }}
    >
      <div className={'sftp-header' + (detached ? ' aux-win-header' : '')}>
        {detached ? (
          <AuxDrag>
            <strong><Icon name="folder" size={15} /> SFTP</strong>
          </AuxDrag>
        ) : (
          <strong><Icon name="folder" size={15} /> SFTP</strong>
        )}
        <div className="aux-win-actions">
          <button className="mini" title="Масштаб" onClick={reset}>{Math.round(zoom * 100)}%</button>
          <button
            className={'mini' + (dualPane ? ' on' : '')}
            title="Двухпанельный режим (локальная ФС + сервер)"
            onClick={toggleDual}
          >
            ⇄
          </button>
          {!detached && (
            <button className="mini" title="Открепить в отдельное окно" onClick={() => void detach()}>
              <Icon name="external" size={14} />
            </button>
          )}
          {detached ? (
            <WindowSysButtons />
          ) : (
            <button className="icon-btn" onClick={onClose} title="Закрыть">
              <Icon name="close" />
            </button>
          )}
        </div>
      </div>

      {/* ---- Локальная панель (двухпанельный режим) ---- */}
      {dualPane && (
        <div
          className={'sftp-local' + (localDragOver ? ' drag-over' : '')}
          onDragOver={(e) => {
            e.preventDefault()
            if (e.dataTransfer.types.includes('x-remote-path')) setLocalDragOver(true)
          }}
          onDragLeave={() => setLocalDragOver(false)}
          onDrop={onLocalDrop}
        >
          <div className="sftp-subhead">Этот компьютер</div>
          <div className="sftp-path">
            <button className="mini" title="Вверх" onClick={() => window.api.localfs.parent(localPath).then(loadLocal)}>
              ↑
            </button>
            <input
              value={localPath}
              onChange={(e) => setLocalPath(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && loadLocal((e.target as HTMLInputElement).value)}
            />
            <button className="mini" title="Домой" onClick={() => window.api.localfs.home().then(loadLocal)}>
              ⌂
            </button>
          </div>
          <div className="sftp-list local">
            {localEntries.map((e) => (
              <div
                key={e.name}
                className="sftp-row"
                draggable={e.type === 'file'}
                onDragStart={(ev) => ev.dataTransfer.setData('x-local-path', joinLocal(localPath, e.name))}
                onDoubleClick={() => e.type === 'dir' && loadLocal(joinLocal(localPath, e.name))}
              >
                <span className="sftp-icon">
                  <Icon name={e.type === 'dir' ? 'folder' : 'file'} size={15} style={{ color: e.type === 'dir' ? 'var(--accent)' : 'var(--muted)' }} />
                </span>
                <span className="sftp-name">{e.name}</span>
                <span className="sftp-size">{e.type === 'file' ? fmtSize(e.size) : ''}</span>
                {e.type === 'file' && (
                  <span className="sftp-row-actions">
                    <button className="mini" title="Загрузить на сервер →" onClick={() => uploadLocalEntry(e)}>
                      →
                    </button>
                  </span>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* ---- Удалённая панель ---- */}
      {dualPane && <div className="sftp-subhead">Сервер</div>}
      <div className="sftp-path">
        <button className="mini" title="Вверх" onClick={() => load(parentOfRemote(path))}>
          <Icon name="up-dir" size={15} />
        </button>
        <input
          value={path}
          onChange={(e) => setPath(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && load((e.target as HTMLInputElement).value)}
        />
        <button className="mini" title="Обновить" onClick={() => load(path)}>
          <Icon name="refresh" size={15} />
        </button>
      </div>

      {/* ---- Хлебные крошки пути (кликабельны) ---- */}
      {remoteCrumbs(path).length > 0 && (
        <div className="sftp-crumbs">
          {remoteCrumbs(path).map((c, i, arr) => (
            <span key={c.path} className="crumb-wrap">
              <button
                className={'crumb' + (i === arr.length - 1 ? ' current' : '')}
                onClick={() => load(c.path)}
                title={c.path}
              >
                {c.label}
              </button>
              {i < arr.length - 1 && <span className="crumb-sep">/</span>}
            </span>
          ))}
        </div>
      )}

      <div className="sftp-toolbar">
        <button className="secondary" onClick={uploadDialog} title="Загрузить файлы на сервер">
          ⬆ Файлы
        </button>
        <button className="secondary" onClick={uploadFolderDialog} title="Загрузить папку (рекурсивно)">
          ⬆ Папка
        </button>
        <button className="secondary" onClick={mkdir}>
          + Папка
        </button>
      </div>

      {error && <div className="sftp-error" onClick={() => setError(null)}>{error}</div>}

      {editList.length > 0 && (
        <div className="sftp-edits">
          {editList.map((ed) => (
            <div key={ed.remotePath} className="sftp-edit-row">
              <span className="edit-state" title={ed.error}>
                {ed.state === 'uploading' ? '⬆' : ed.state === 'synced' ? '✓' : ed.state === 'error' ? '⚠' : '✎'}
              </span>
              <span className="sftp-name">{ed.remotePath.split('/').pop()}</span>
              <span className="edit-label">
                {ed.state === 'uploading'
                  ? 'заливка…'
                  : ed.state === 'synced'
                    ? 'сохранено'
                    : ed.state === 'error'
                      ? 'ошибка'
                      : 'редактируется'}
              </span>
              <button
                className="mini"
                title="Перестать следить"
                onClick={() => window.api.sftp.editStop(sessionId, ed.remotePath)}
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      )}

      <div
        className={'sftp-list remote' + (dragOver ? ' drag-over' : '')}
        onDragOver={(e) => {
          e.preventDefault()
          setDragOver(true)
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={onRemoteDrop}
      >
        {loading && <div className="hint">Загрузка…</div>}
        {!loading &&
          entries.map((e) => (
            <div
              key={e.name}
              className="sftp-row"
              draggable={e.type === 'file'}
              onDragStart={(ev) => {
                ev.dataTransfer.setData('x-remote-path', joinRemote(path, e.name))
                window.api.sftp.startDrag(sessionId, joinRemote(path, e.name))
              }}
              onDoubleClick={() => {
                if (e.type === 'dir') return load(joinRemote(path, e.name))
                // Текстовый файл → встроенный редактор; иначе скачивание.
                if (e.type === 'file' && onOpenInEditor && isTextFile(e.name)) {
                  return onOpenInEditor(joinRemote(path, e.name))
                }
                return dualPane ? downloadToLocal(e) : download(e)
              }}
            >
              <span className="sftp-icon">
                <Icon name={e.type === 'dir' ? 'folder' : 'file'} size={15} style={{ color: e.type === 'dir' ? 'var(--accent)' : 'var(--muted)' }} />
              </span>
              {renaming === e.name ? (
                <input
                  className="sftp-rename"
                  autoFocus
                  value={renameValue}
                  onClick={(ev) => ev.stopPropagation()}
                  onChange={(ev) => setRenameValue(ev.target.value)}
                  onBlur={() => void commitRename(e.name)}
                  onKeyDown={(ev) => {
                    if (ev.key === 'Enter') void commitRename(e.name)
                    if (ev.key === 'Escape') setRenaming(null)
                  }}
                />
              ) : (
                <span className="sftp-name">{e.name}</span>
              )}
              <span className="sftp-size">{e.type === 'file' ? fmtSize(e.size) : ''}</span>
              <span className="sftp-row-actions">
                {dualPane && e.type !== 'dir' && (
                  <button className="mini" title="Скачать на ПК" onClick={() => downloadToLocal(e)}>
                    <Icon name="back" size={14} />
                  </button>
                )}
                {e.type === 'file' && onOpenInEditor && isTextFile(e.name) && (
                  <button className="mini" title="Открыть во встроенном редакторе" onClick={() => onOpenInEditor(joinRemote(path, e.name))}>
                    <Icon name="editor" size={14} />
                  </button>
                )}
                {e.type === 'file' && (
                  <button className="mini" title="Открыть во внешнем редакторе" onClick={() => editRemote(e)}>
                    <Icon name="external" size={14} />
                  </button>
                )}
                <button className="mini" title="Скачать на этот компьютер" onClick={() => download(e)}>
                  <Icon name="import" size={14} />
                </button>
                <button className="mini" title="Переименовать" onClick={() => startRename(e)}>
                  <Icon name="edit" size={14} />
                </button>
                <button className="mini danger" title="Удалить" onClick={() => remove(e)}>
                  <Icon name="trash" size={14} />
                </button>
              </span>
            </div>
          ))}
      </div>

      {/* ---- Очередь передач ---- */}
      {transfers.length > 0 && (
        <div className="sftp-queue">
          <div className="sftp-queue-head">
            <span>Передачи ({activeTransfers.length} активны)</span>
            <button className="mini" title="Очистить завершённые" onClick={clearFinished}>
              Очистить
            </button>
          </div>
          <div className="sftp-queue-list">
            {transfers.map((t) => (
              <div key={t.id} className={'sftp-queue-item ' + t.state}>
                <span className="q-dir">{t.direction === 'upload' ? '⬆' : '⬇'}</span>
                <div className="q-info">
                  <div className="q-name" title={t.error || t.filename}>
                    {t.filename}
                    {t.state === 'error' && <span className="q-err"> — {t.error}</span>}
                  </div>
                  {(t.state === 'active' || t.state === 'queued' || t.state === 'paused') && (
                    <div className="bar">
                      <div
                        className="bar-fill"
                        style={{ width: t.size ? `${Math.min(100, (t.transferred / t.size) * 100)}%` : '0%' }}
                      />
                    </div>
                  )}
                </div>
                <span className="q-state">
                  {t.state === 'done'
                    ? '✓'
                    : t.state === 'error'
                      ? '⚠'
                      : t.state === 'canceled'
                        ? '⊘'
                        : t.state === 'queued'
                          ? 'ожидание'
                          : t.state === 'paused'
                            ? 'пауза'
                            : [
                                t.size
                                  ? `${fmtSize(t.transferred)} / ${fmtSize(t.size)}`
                                  : fmtSize(t.transferred),
                                fmtSpeed(t.speedBps),
                                fmtEta(t.size, t.transferred, t.speedBps),
                              ]
                                .filter(Boolean)
                                .join(' · ')}
                </span>
                <span className="q-actions">
                  {t.state === 'active' && (
                    <button className="mini" title="Пауза" onClick={() => pauseTransfer(t.id)}>
                      ❚❚
                    </button>
                  )}
                  {t.state === 'paused' && (
                    <button className="mini" title="Продолжить" onClick={() => resumeTransfer(t.id)}>
                      ▶
                    </button>
                  )}
                  {(t.state === 'error' || t.state === 'canceled') && (
                    <button className="mini" title="Повторить" onClick={() => retryTransfer(t)}>
                      ↻
                    </button>
                  )}
                  {(t.state === 'queued' || t.state === 'active' || t.state === 'paused') && (
                    <button className="mini danger" title="Отменить" onClick={() => cancelTransfer(t.id)}>
                      ✕
                    </button>
                  )}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
