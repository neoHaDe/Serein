import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { SftpEntry, LocalEntry, TransferItem, RemoteEditStatus } from '../../shared/types'
import { isImageFile, isTextFile } from '../editorLang'
import { Icon } from './Icon'
import { openAuxWindow, sanitizeWindowLabel } from '../auxWindows'
import { reattachSftp } from '../reattach'
import { useCtrlWheelZoom } from '../useCtrlWheelZoom'
import { AuxDrag, WindowSysButtons } from './WindowChrome'
import { AuxReattachButton } from './AuxReattachButton'
import { useSettings } from '../SettingsContext'
import {
  SFTP_COL_LABEL,
  SFTP_COL_MIN,
  SFTP_COL_ORDER,
  type SftpColId,
  fileExt,
  fmtMtime,
  gridTemplate,
  mergeColOn,
  mergeColWidths,
  mergeSort,
  nextSort,
  sortEntries,
  visibleCols,
  clickSelect,
  entryKind,
  fmtPerms,
} from '../sftpExplorer'
import { errText } from '../errText'

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
  /** Сервер вкладки — нужен, чтобы после перезапуска снова открыть откреплённое окно. */
  serverId?: string
  onClose: () => void
  width: number
  closing: boolean
  detached?: boolean
  /** Занять всю область workspace (не полоска 380px). */
  fill?: boolean
  onOpenInEditor?: (remotePath: string) => void
}

function fmtSize(n: number): string {
  if (n < 1024) return `${n} Б`
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} КБ`
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} МБ`
  return `${(n / 1024 ** 3).toFixed(2)} ГБ`
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

function fmtMode(mode: number): string {
  return (mode & 0o777).toString(8).padStart(3, '0')
}

function isHiddenName(name: string): boolean {
  return name.startsWith('.') && name !== '.' && name !== '..'
}

function filterEntries<T extends { name: string }>(entries: T[], showHidden: boolean, query: string): T[] {
  const q = query.trim().toLowerCase()
  return entries.filter((e) => {
    if (!showHidden && isHiddenName(e.name)) return false
    if (q && !e.name.toLowerCase().includes(q)) return false
    return true
  })
}

function isDirEntry(e: SftpEntry): boolean {
  return e.type === 'dir' || (e.type === 'link' && e.linkType === 'dir')
}

function isFileLike(e: SftpEntry): boolean {
  return e.type === 'file' || (e.type === 'link' && e.linkType !== 'dir')
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

function pointIn(el: HTMLElement | null, x: number, y: number): boolean {
  if (!el) return false
  const r = el.getBoundingClientRect()
  return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}

function isSereinDnd(p: string): boolean {
  return p.replace(/\\/g, '/').toLowerCase().includes('/serein-dnd/')
}

function eventPoint(pos: { x: number; y: number }): { x: number; y: number } {
  const f = window.devicePixelRatio || 1
  return { x: pos.x / f, y: pos.y / f }
}

function localBaseName(p: string): string {
  const n = p.replace(/\\/g, '/').replace(/\/+$/, '')
  const i = n.lastIndexOf('/')
  return i >= 0 ? n.slice(i + 1) : n
}

function OverwriteAsk({
  names,
  onYes,
  onNo,
}: {
  names: string[]
  onYes: () => void
  onNo: () => void
}): JSX.Element {
  const msg =
    names.length === 1
      ? `«${names[0]}» уже есть на сервере. Заменить?`
      : `На сервере уже есть ${names.length} из выбранных: ${names.slice(0, 8).join(', ')}${names.length > 8 ? '…' : ''}. Заменить?`
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onNo()}>
      <div className="modal sftp-props-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Файл уже есть</h2>
        <p className="hint">{msg}</p>
        <div className="modal-actions">
          <button type="button" onClick={onNo}>
            Отмена
          </button>
          <button type="button" className="primary" onClick={onYes}>
            Заменить
          </button>
        </div>
      </div>
    </div>
  )
}

function ExplorerHead({
  cols,
  sortCol,
  sortDir,
  onSort,
  onResize,
  onContext,
}: {
  cols: SftpColId[]
  sortCol: SftpColId
  sortDir: 'asc' | 'desc'
  onSort: (id: SftpColId) => void
  onResize: (id: SftpColId, e: React.MouseEvent) => void
  onContext: (e: React.MouseEvent) => void
}): JSX.Element {
  return (
    <div className="sftp-row sftp-row-head" onContextMenu={onContext}>
      {cols.map((id) => (
        <button
          key={id}
          type="button"
          className={'sftp-th' + (sortCol === id ? ' sorted' : '')}
          title="Сортировка · ПКМ — столбцы"
          onClick={() => onSort(id)}
        >
          <span className="sftp-th-label">{SFTP_COL_LABEL[id]}</span>
          {sortCol === id ? (
            <Icon name={sortDir === 'asc' ? 'chevron-up' : 'chevron-down'} size={12} />
          ) : null}
          <span
            className="sftp-col-resizer"
            onMouseDown={(e) => onResize(id, e)}
            onClick={(e) => e.stopPropagation()}
          />
        </button>
      ))}
    </div>
  )
}

function CtxItem({ label, danger, onPick }: { label: string; danger?: boolean; onPick: () => void }): JSX.Element {
  return (
    <button type="button" className={'sftp-ctx-item' + (danger ? ' danger' : '')} onClick={onPick}>
      {label}
    </button>
  )
}

function AnchoredMenu({
  x,
  y,
  className,
  children,
}: {
  x: number
  y: number
  className: string
  children: React.ReactNode
}): JSX.Element {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: x, top: y })
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const pad = 8
    const { width, height } = el.getBoundingClientRect()
    let left = x
    let top = y
    if (left + width > window.innerWidth - pad) left = window.innerWidth - pad - width
    if (left < pad) left = pad
    if (top + height > window.innerHeight - pad) top = y - height
    if (top < pad) top = pad
    if (top + height > window.innerHeight - pad) top = Math.max(pad, window.innerHeight - pad - height)
    setPos({ left, top })
  }, [x, y])
  return (
    <div
      ref={ref}
      className={className}
      style={{ left: pos.left, top: pos.top }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      {children}
    </div>
  )
}

function PropsSheet({
  title,
  rows,
  onClose,
}: {
  title: string
  rows: { k: string; v: string }[]
  onClose: () => void
}): JSX.Element {
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal sftp-props-modal" onClick={(e) => e.stopPropagation()}>
        <h2>{title}</h2>
        <dl className="sftp-props">
          {rows.map((r) => (
            <div key={r.k} className="sftp-props-row">
              <dt>{r.k}</dt>
              <dd title={r.v}>{r.v || '—'}</dd>
            </div>
          ))}
        </dl>
        <div className="modal-actions">
          <button className="primary" onClick={onClose}>
            OK
          </button>
        </div>
      </div>
    </div>
  )
}

export function SftpPanel({ sessionId, serverId, onClose, width, closing, detached, fill, onOpenInEditor }: Props): JSX.Element {
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
  const [showHidden, setShowHidden] = useState(false)
  const [filter, setFilter] = useState('')
  const [chmodEntry, setChmodEntry] = useState<SftpEntry | null>(null)
  const [chmodMode, setChmodMode] = useState(0o644)
  const { zoom, ref: zoomRef, reset } = useCtrlWheelZoom('serein.sftp.zoom')
  const { settings, update } = useSettings()
  const colOn = mergeColOn(settings.sftpColOn)
  const colWidthsBase = mergeColWidths(settings.sftpColWidths)
  const [liveWidths, setLiveWidths] = useState<Record<SftpColId, number> | null>(null)
  const colWidths = liveWidths ?? colWidthsBase
  const sort = mergeSort(settings.sftpSortCol, settings.sftpSortDir)
  const remoteCols = visibleCols(colOn, false)
  const localCols = visibleCols(colOn, true)
  const [colMenu, setColMenu] = useState<{ x: number; y: number } | null>(null)
  const [ctxMenu, setCtxMenu] = useState<
    { pane: 'remote' | 'local'; x: number; y: number; names: string[] } | null
  >(null)
  const [selRemote, setSelRemote] = useState<string[]>([])
  const [selLocal, setSelLocal] = useState<string[]>([])
  const [anchorRemote, setAnchorRemote] = useState<string | null>(null)
  const [anchorLocal, setAnchorLocal] = useState<string | null>(null)
  const [propsOpen, setPropsOpen] = useState<{ pane: 'remote' | 'local'; names: string[] } | null>(null)

  const pathRef = useRef(path)
  pathRef.current = path
  const localPathRef = useRef(localPath)
  localPathRef.current = localPath
  const sessionIdRef = useRef(sessionId)
  sessionIdRef.current = sessionId
  const entriesRef = useRef(entries)
  entriesRef.current = entries
  const panelRef = useRef<HTMLDivElement | null>(null)
  const localPaneRef = useRef<HTMLDivElement | null>(null)
  const loadRemoteRef = useRef<(t: string, silent?: boolean) => void>(() => {})
  const loadLocalRef = useRef<(t: string) => void>(() => {})
  const uploadToRemoteRef = useRef<(paths: string[]) => Promise<void>>(async () => {})
  const overwriteResolve = useRef<((ok: boolean) => void) | null>(null)
  const [overwriteNames, setOverwriteNames] = useState<string[] | null>(null)

  const askOverwrite = (names: string[]): Promise<boolean> =>
    new Promise((resolve) => {
      overwriteResolve.current?.(false)
      overwriteResolve.current = resolve
      setOverwriteNames(names)
    })

  const answerOverwrite = (ok: boolean): void => {
    overwriteResolve.current?.(ok)
    overwriteResolve.current = null
    setOverwriteNames(null)
  }

  const load = useCallback(
    async (target: string, silent = false) => {
      if (!silent) {
        setLoading(true)
        setError(null)
      }
      try {
        const res = await window.api.sftp.list(sessionId, target)
        setPath(res.path)
        setEntries(res.entries)
        if (!silent) {
          setSelRemote([])
          setAnchorRemote(null)
          setCtxMenu(null)
        }
      } catch (e) {
        setError(errText(e))
      } finally {
        if (!silent) setLoading(false)
      }
    },
    [sessionId]
  )

  const loadLocal = useCallback(async (target: string) => {
    try {
      const res = await window.api.localfs.list(target)
      setLocalPath(res.path)
      setLocalEntries(res.entries)
      setSelLocal([])
      setAnchorLocal(null)
    } catch (e) {
      setError(errText(e))
    }
  }, [])
  loadRemoteRef.current = load
  loadLocalRef.current = loadLocal

  const uploadToRemote = async (paths: string[]): Promise<void> => {
    if (!paths.length) return
    const names = [...new Set(paths.map(localBaseName).filter(Boolean))]
    let clash: string[] = []
    try {
      clash = await window.api.sftp.nameConflicts(sessionIdRef.current, pathRef.current, names)
    } catch {
      clash = names.filter((n) => entriesRef.current.some((e) => e.name === n))
    }
    if (clash.length && !(await askOverwrite(clash))) return
    try {
      await window.api.sftp.uploadPaths(sessionIdRef.current, pathRef.current, paths)
    } catch (e) {
      setError(errText(e))
    }
  }
  uploadToRemoteRef.current = uploadToRemote

  useEffect(() => {
    const win = getCurrentWindow()
    let off: (() => void) | undefined
    let dead = false
    void win
      .onDragDropEvent((event) => {
        const p = event.payload
        if (p.type === 'leave') {
          setDragOver(false)
          setLocalDragOver(false)
          return
        }
        const pos = 'position' in p ? eventPoint(p.position) : null
        if (p.type === 'enter' || p.type === 'over') {
          if (!pos) return
          if (pointIn(localPaneRef.current, pos.x, pos.y)) {
            setLocalDragOver(true)
            setDragOver(false)
          } else if (pointIn(panelRef.current, pos.x, pos.y)) {
            setDragOver(true)
            setLocalDragOver(false)
          } else {
            setDragOver(false)
            setLocalDragOver(false)
          }
          return
        }
        if (p.type !== 'drop' || !pos) return
        setDragOver(false)
        setLocalDragOver(false)
        const paths = p.paths.filter(Boolean)
        if (!paths.length) return
        if (pointIn(localPaneRef.current, pos.x, pos.y) && localPathRef.current) {
          void window.api.localfs
            .copyInto(paths, localPathRef.current)
            .then(() => loadLocalRef.current(localPathRef.current))
            .catch((e) => setError(errText(e)))
          return
        }
        if (!pointIn(panelRef.current, pos.x, pos.y)) return
        const upload = paths.filter((x) => !isSereinDnd(x))
        if (!upload.length) return
        window.setTimeout(() => {
          void uploadToRemoteRef.current(upload)
        }, 0)
      })
      .then((u) => {
        if (dead) u()
        else off = u
      })
    return () => {
      dead = true
      off?.()
    }
  }, [])

  useEffect(() => {
    load('.')
  }, [load])

  useEffect(() => {
    void window.api.settings.get().then((s) => setShowHidden(!!s.sftpShowHidden))
  }, [])

  useEffect(() => {
    if (!colMenu && !ctxMenu) return
    const close = (e: MouseEvent): void => {
      if (e.button === 2) return
      const t = e.target as HTMLElement | null
      if (t?.closest('.sftp-ctx-menu, .sftp-col-menu')) return
      setColMenu(null)
      setCtxMenu(null)
    }
    const id = window.setTimeout(() => document.addEventListener('mousedown', close), 0)
    return () => {
      window.clearTimeout(id)
      document.removeEventListener('mousedown', close)
    }
  }, [colMenu, ctxMenu])

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
        if (isSereinDnd(item.localPath)) return
        if (item.direction === 'upload') load(pathRef.current, true)
        else if (localPathRef.current) loadLocal(localPathRef.current)
      } else if (item.state === 'canceled' && localPathRef.current && !isSereinDnd(item.localPath)) {
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
      const paths = await window.api.files.pick({
        multiple: true,
        title: 'Файлы для загрузки на сервер'
      })
      await uploadToRemote(paths)
    } catch (e) {
      setError(errText(e))
    }
  }
  const uploadFolderDialog = async (): Promise<void> => {
    try {
      const paths = await window.api.files.pick({
        directory: true,
        title: 'Папка для загрузки на сервер'
      })
      await uploadToRemote(paths)
    } catch (e) {
      setError(errText(e))
    }
  }

  const editRemote = async (entry: SftpEntry): Promise<void> => {
    try {
      await window.api.sftp.edit(sessionId, joinRemote(path, entry.name))
    } catch (e) {
      setError(errText(e))
    }
  }

  const mkdir = async (): Promise<void> => {
    const name = prompt('Имя новой папки:')
    if (!name) return
    try {
      await window.api.sftp.mkdir(sessionId, joinRemote(path, name))
      load(path)
    } catch (e) {
      setError(errText(e))
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
      setError(errText(e))
    }
  }

  const toggleHidden = (): void => {
    const next = !showHidden
    setShowHidden(next)
    void window.api.settings.set({ sftpShowHidden: next })
  }

  const openChmod = (entry: SftpEntry): void => {
    setChmodEntry(entry)
    setChmodMode(entry.mode & 0o777)
  }

  const applyChmod = async (): Promise<void> => {
    if (!chmodEntry) return
    try {
      await window.api.sftp.chmod(sessionId, joinRemote(path, chmodEntry.name), chmodMode)
      setChmodEntry(null)
      load(path)
    } catch (e) {
      setError(errText(e))
    }
  }

  const visibleRemote = useMemo(
    () => sortEntries(filterEntries(entries, showHidden, filter), sort),
    [entries, showHidden, filter, sort]
  )
  const visibleLocal = useMemo(
    () => sortEntries(filterEntries(localEntries, showHidden, filter), sort),
    [localEntries, showHidden, filter, sort]
  )

  const onSortCol = (id: SftpColId): void => {
    const next = nextSort(sort, id)
    update({ sftpSortCol: next.col, sftpSortDir: next.dir })
  }

  const onResizeCol = (id: SftpColId, e: React.MouseEvent): void => {
    e.preventDefault()
    e.stopPropagation()
    const x0 = e.clientX
    const w0 = colWidths[id]
    const cur = { ...colWidths }
    const move = (ev: MouseEvent): void => {
      cur[id] = Math.max(SFTP_COL_MIN[id], Math.round(w0 + (ev.clientX - x0)))
      setLiveWidths({ ...cur })
    }
    const up = (): void => {
      document.removeEventListener('mousemove', move)
      document.removeEventListener('mouseup', up)
      update({ sftpColWidths: { ...cur } })
      setLiveWidths(null)
    }
    document.addEventListener('mousemove', move)
    document.addEventListener('mouseup', up)
  }

  const onHeadContext = (e: React.MouseEvent): void => {
    e.preventDefault()
    setCtxMenu(null)
    setColMenu({ x: e.clientX, y: e.clientY })
  }

  const toggleCol = (id: SftpColId): void => {
    if (id === 'name') return
    update({ sftpColOn: { ...colOn, [id]: !colOn[id] } })
  }

  const remoteNames = useMemo(() => visibleRemote.map((e) => e.name), [visibleRemote])
  const localNames = useMemo(() => visibleLocal.map((e) => e.name), [visibleLocal])
  const pickRemote = (names: string[]): SftpEntry[] =>
    names.map((n) => entries.find((e) => e.name === n)).filter((e): e is SftpEntry => !!e)
  const pickLocal = (names: string[]): LocalEntry[] =>
    names.map((n) => localEntries.find((e) => e.name === n)).filter((e): e is LocalEntry => !!e)

  const onRemoteMouseDown = (entry: SftpEntry, ev: React.MouseEvent): void => {
    if (ev.button !== 0) return
    ;(ev.currentTarget.parentElement as HTMLElement | null)?.focus()
    const r = clickSelect(remoteNames, selRemote, entry.name, { ctrl: ev.ctrlKey || ev.metaKey, shift: ev.shiftKey }, anchorRemote)
    setSelRemote(r.selected)
    setAnchorRemote(r.anchor)
  }
  const onLocalMouseDown = (entry: LocalEntry, ev: React.MouseEvent): void => {
    if (ev.button !== 0) return
    ;(ev.currentTarget.parentElement as HTMLElement | null)?.focus()
    const r = clickSelect(localNames, selLocal, entry.name, { ctrl: ev.ctrlKey || ev.metaKey, shift: ev.shiftKey }, anchorLocal)
    setSelLocal(r.selected)
    setAnchorLocal(r.anchor)
  }

  const openRemoteCtx = (entry: SftpEntry, ev: React.MouseEvent): void => {
    ev.preventDefault()
    ev.stopPropagation()
    setColMenu(null)
    let names = selRemote
    if (!names.includes(entry.name)) {
      names = [entry.name]
      setSelRemote(names)
      setAnchorRemote(entry.name)
    }
    setCtxMenu({ pane: 'remote', x: ev.clientX, y: ev.clientY, names })
  }
  const openLocalCtx = (entry: LocalEntry, ev: React.MouseEvent): void => {
    ev.preventDefault()
    ev.stopPropagation()
    setColMenu(null)
    let names = selLocal
    if (!names.includes(entry.name)) {
      names = [entry.name]
      setSelLocal(names)
      setAnchorLocal(entry.name)
    }
    setCtxMenu({ pane: 'local', x: ev.clientX, y: ev.clientY, names })
  }

  const closeMenus = (): void => {
    setCtxMenu(null)
    setColMenu(null)
  }

  const downloadMany = async (items: SftpEntry[]): Promise<void> => {
    if (!items.length) return
    try {
      const first = await window.api.sftp.download(sessionId, joinRemote(path, items[0].name))
      if (!first.saved || !first.path) return
      for (const item of items.slice(1)) {
        await window.api.sftp.downloadTo(sessionId, joinRemote(path, item.name), first.path)
      }
    } catch (e) {
      setError(errText(e))
    }
  }

  const removeMany = async (items: SftpEntry[]): Promise<void> => {
    if (!items.length) return
    const msg = items.length === 1 ? `Удалить «${items[0].name}»?` : `Удалить ${items.length} элементов?`
    if (!confirm(msg)) return
    try {
      for (const item of items) {
        await window.api.sftp.remove(sessionId, joinRemote(path, item.name), item.type === 'dir')
      }
      setSelRemote([])
      load(path)
    } catch (e) {
      setError(errText(e))
      load(path)
    }
  }

  const openBuiltin = (items: SftpEntry[]): void => {
    if (!onOpenInEditor) return
    for (const item of items) {
      if (isFileLike(item) && (isTextFile(item.name) || isImageFile(item.name))) {
        onOpenInEditor(joinRemote(path, item.name))
      }
    }
  }

  const openExternalMany = async (items: SftpEntry[]): Promise<void> => {
    for (const item of items) {
      if (isFileLike(item) && !isImageFile(item.name)) await editRemote(item)
    }
  }

  const openItems = (items: SftpEntry[]): void => {
    if (items.length === 1 && items[0].type === 'link' && items[0].linkType === 'broken') {
      setError('Сломанная ссылка' + (items[0].target ? ': ' + items[0].target : ''))
      return
    }
    if (items.length === 1 && isDirEntry(items[0])) {
      load(joinRemote(path, items[0].name))
      return
    }
    const files = items.filter(isFileLike)
    const editable = files.filter((e) => isTextFile(e.name) || isImageFile(e.name))
    if (onOpenInEditor && editable.length) {
      openBuiltin(editable)
      return
    }
    void downloadMany(files.length ? files : items)
  }

  const downloadToLocalMany = async (items: SftpEntry[]): Promise<void> => {
    if (!localPath) return
    for (const item of items) {
      await window.api.sftp.downloadTo(sessionId, joinRemote(path, item.name), localPath)
    }
  }

  const uploadLocalMany = async (items: LocalEntry[]): Promise<void> => {
    const paths = items.map((e) => joinLocal(localPath, e.name))
    await uploadToRemote(paths)
  }

  const propsRowsFor = (pane: 'remote' | 'local', names: string[]): { title: string; rows: { k: string; v: string }[] } => {
    if (pane === 'remote') {
      const items = pickRemote(names)
      if (items.length === 1) {
        const e = items[0]
        const rows = [
          { k: 'Имя', v: e.name },
          { k: 'Тип', v: entryKind(e) },
          { k: 'Расположение', v: path },
          { k: 'Размер', v: fmtSize(e.size) },
          { k: 'Изменён', v: fmtMtime(e.mtime) },
          { k: 'Права', v: fmtPerms(e.mode) },
        ]
        if (e.type === 'link' && e.target) rows.push({ k: 'Ссылка', v: e.target })
        return { title: 'Свойства', rows }
      }
      const dirs = items.filter(isDirEntry).length
      const total = items.reduce((s, e) => s + (e.size || 0), 0)
      return {
        title: items.length + ' элементов',
        rows: [
          { k: 'Выделено', v: String(items.length) },
          { k: 'Папок', v: String(dirs) },
          { k: 'Файлов', v: String(items.length - dirs) },
          { k: 'Расположение', v: path },
          { k: 'Суммарный размер', v: fmtSize(total) },
        ],
      }
    }
    const items = pickLocal(names)
    if (items.length === 1) {
      const e = items[0]
      return {
        title: 'Свойства',
        rows: [
          { k: 'Имя', v: e.name },
          { k: 'Тип', v: entryKind(e) },
          { k: 'Расположение', v: localPath },
          { k: 'Размер', v: fmtSize(e.size) },
          { k: 'Изменён', v: fmtMtime(e.mtime) },
        ],
      }
    }
    const dirs = items.filter((e) => e.type === 'dir').length
    const total = items.reduce((s, e) => s + (e.size || 0), 0)
    return {
      title: items.length + ' элементов',
      rows: [
        { k: 'Выделено', v: String(items.length) },
        { k: 'Папок', v: String(dirs) },
        { k: 'Файлов', v: String(items.length - dirs) },
        { k: 'Расположение', v: localPath },
        { k: 'Суммарный размер', v: fmtSize(total) },
      ],
    }
  }

  const onRemoteKey = (ev: React.KeyboardEvent): void => {
    if ((ev.target as HTMLElement).tagName === 'INPUT') return
    if (ev.key === 'a' && (ev.ctrlKey || ev.metaKey)) {
      ev.preventDefault()
      setSelRemote(remoteNames)
      return
    }
    if (ev.key === 'Escape') {
      setSelRemote([])
      closeMenus()
      return
    }
    if (ev.key === 'Delete') {
      ev.preventDefault()
      void removeMany(pickRemote(selRemote))
      return
    }
    if (ev.key === 'F2' && selRemote.length === 1) {
      const e = pickRemote(selRemote)[0]
      if (e) startRename(e)
      return
    }
    if (ev.key === 'Enter' && selRemote.length) openItems(pickRemote(selRemote))
  }

  const onLocalKey = (ev: React.KeyboardEvent): void => {
    if (ev.key === 'a' && (ev.ctrlKey || ev.metaKey)) {
      ev.preventDefault()
      setSelLocal(localNames)
      return
    }
    if (ev.key === 'Escape') {
      setSelLocal([])
      closeMenus()
    }
  }

  // ---- Drag & drop на удалённую панель (загрузка) ----
  const onRemoteDrop = async (e: React.DragEvent): Promise<void> => {
    e.preventDefault()
    setDragOver(false)
    const localSrc = e.dataTransfer.getData('x-local-path')
    if (localSrc) await uploadToRemote([localSrc])
  }

  // ---- Local actions ----
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
  const ctxRemoteItems = ctxMenu?.pane === 'remote' ? pickRemote(ctxMenu.names) : []
  const ctxLocalItems = ctxMenu?.pane === 'local' ? pickLocal(ctxMenu.names) : []
  const ctxBuiltin = ctxRemoteItems.filter((e) => isFileLike(e) && (isTextFile(e.name) || isImageFile(e.name)))
  const ctxExternal = ctxRemoteItems.filter((e) => isFileLike(e) && !isImageFile(e.name))
  const ctxFiles = ctxRemoteItems.filter(isFileLike)

  const detach = async (): Promise<void> => {
    await openAuxWindow({
      label: 'sftp-' + sanitizeWindowLabel(sessionId),
      query: { sftp: '1', sessionId, serverId: serverId ?? '' },
      title: 'SFTP',
      width: Math.max(width, 480),
      height: 720,
      persist: serverId ? { kind: 'sftp', serverId } : undefined
    })
    onClose()
  }

  return (
    <div
      className={'sftp-panel' + (closing ? ' closing' : '') + (detached ? ' detached' : '') + (fill ? ' fill' : '')}
      ref={(el) => {
        panelRef.current = el
        zoomRef(el)
      }}
      style={detached || fill ? { zoom } : { width, zoom }}
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
          {detached && (
            <AuxReattachButton
              onClick={() => reattachSftp({ sessionId, serverId })}
            />
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
          ref={localPaneRef}
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
          <div
            className="sftp-list local"
            style={{ ['--sftp-grid' as string]: gridTemplate(localCols, colWidths) }}
            tabIndex={0}
            onKeyDown={onLocalKey}
            onMouseDown={(ev) => {
              if (ev.target === ev.currentTarget) {
                setSelLocal([])
                setAnchorLocal(null)
              }
            }}
          >
            <ExplorerHead
              cols={localCols}
              sortCol={sort.col}
              sortDir={sort.dir}
              onSort={onSortCol}
              onResize={onResizeCol}
              onContext={onHeadContext}
            />
            {visibleLocal.map((e) => (
              <div
                key={e.name}
                className={'sftp-row' + (selLocal.includes(e.name) ? ' selected' : '')}
                onMouseDown={(ev) => {
                  ;(ev.currentTarget as HTMLDivElement).draggable = ev.button === 0
                  onLocalMouseDown(e, ev)
                }}
                onDragEnd={(ev) => {
                  ;(ev.currentTarget as HTMLDivElement).draggable = false
                }}
                onContextMenu={(ev) => openLocalCtx(e, ev)}
                onDragStart={(ev) => ev.dataTransfer.setData('x-local-path', joinLocal(localPath, e.name))}
                onDoubleClick={() => e.type === 'dir' && loadLocal(joinLocal(localPath, e.name))}
              >
                {localCols.map((col) => {
                  if (col === 'name') {
                    return (
                      <span key={col} className="sftp-namecell">
                        <span className="sftp-icon">
                          <Icon name={e.type === 'dir' ? 'folder' : 'file'} size={15} style={{ color: e.type === 'dir' ? 'var(--accent)' : 'var(--muted)' }} />
                        </span>
                        <span className="sftp-name">{e.name}</span>
                      </span>
                    )
                  }
                  if (col === 'ext') return <span key={col} className="sftp-td sftp-ext">{fileExt(e.name, e.type === 'dir')}</span>
                  if (col === 'size') return <span key={col} className="sftp-td sftp-size">{fmtSize(e.size)}</span>
                  if (col === 'mtime') return <span key={col} className="sftp-td sftp-mtime">{fmtMtime(e.mtime)}</span>
                  return null
                })}
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
        <span className="sftp-filter">
          <Icon name="search" size={14} />
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Фильтр…"
          />
        </span>
        <button
          className={'mini' + (showHidden ? ' on' : '')}
          title={showHidden ? 'Скрыть файлы с точкой' : 'Показать скрытые файлы'}
          onClick={toggleHidden}
        >
          <Icon name={showHidden ? 'eye' : 'eye-off'} size={14} />
        </button>
      </div>

      {chmodEntry && (
        <div className="chmod-pop">
          <span className="chmod-name" title={chmodEntry.name}>
            {chmodEntry.name} · {fmtMode(chmodMode)}
          </span>
          <div className="chmod-bits">
            {(['u', 'g', 'o'] as const).map((who, wi) => (
              <span key={who} className="chmod-who">
                <span className="chmod-who-l">{who}</span>
                {(['r', 'w', 'x'] as const).map((bit, bi) => {
                  const shift = 8 - (wi * 3 + bi)
                  const on = (chmodMode & (1 << shift)) !== 0
                  return (
                    <button
                      key={bit}
                      type="button"
                      className={'chmod-bit' + (on ? ' on' : '')}
                      title={who + bit}
                      onClick={() => setChmodMode((m) => m ^ (1 << shift))}
                    >
                      {bit}
                    </button>
                  )
                })}
              </span>
            ))}
          </div>
          <button className="secondary" onClick={() => void applyChmod()}>
            OK
          </button>
          <button className="mini" onClick={() => setChmodEntry(null)}>
            ✕
          </button>
        </div>
      )}

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
        style={{ ['--sftp-grid' as string]: gridTemplate(remoteCols, colWidths) }}
        tabIndex={0}
        onKeyDown={onRemoteKey}
        onMouseDown={(ev) => {
          if (ev.target === ev.currentTarget) {
            setSelRemote([])
            setAnchorRemote(null)
          }
        }}
        onDragOver={(e) => {
          e.preventDefault()
          setDragOver(true)
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={onRemoteDrop}
      >
        {loading && <div className="hint">Загрузка…</div>}
        {!loading && (
          <ExplorerHead
            cols={remoteCols}
            sortCol={sort.col}
            sortDir={sort.dir}
            onSort={onSortCol}
            onResize={onResizeCol}
            onContext={onHeadContext}
          />
        )}
        {!loading &&
          visibleRemote.map((e) => (
            <div
              key={e.name}
              className={'sftp-row' + (selRemote.includes(e.name) ? ' selected' : '')}
              onMouseDown={(ev) => {
                onRemoteMouseDown(e, ev)
                if (ev.button !== 0 || renaming === e.name || ev.ctrlKey || ev.metaKey || ev.shiftKey) return
                const names = selRemote.includes(e.name) && selRemote.length > 1 ? selRemote : [e.name]
                const x0 = ev.clientX
                const y0 = ev.clientY
                const onMove = (me: MouseEvent): void => {
                  if (Math.abs(me.clientX - x0) < 6 && Math.abs(me.clientY - y0) < 6) return
                  window.removeEventListener('mousemove', onMove)
                  window.removeEventListener('mouseup', onUp)
                  void window.api.sftp
                    .startOsDrag(
                      sessionId,
                      names.map((n) => joinRemote(path, n))
                    )
                    .catch((err) => setError((err as Error).message))
                }
                const onUp = (): void => {
                  window.removeEventListener('mousemove', onMove)
                  window.removeEventListener('mouseup', onUp)
                }
                window.addEventListener('mousemove', onMove)
                window.addEventListener('mouseup', onUp)
              }}
              onContextMenu={(ev) => openRemoteCtx(e, ev)}
              onDoubleClick={() => {
                const items = selRemote.includes(e.name) && selRemote.length > 1 ? pickRemote(selRemote) : [e]
                openItems(items)
              }}
            >
              {remoteCols.map((col) => {
                if (col === 'name') {
                  return (
                    <span key={col} className="sftp-namecell">
                      <span className="sftp-icon">
                        <Icon
                          name={isDirEntry(e) ? 'folder' : e.type === 'link' ? 'link' : 'file'}
                          size={15}
                          style={{ color: isDirEntry(e) ? 'var(--accent)' : 'var(--muted)' }}
                        />
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
                        <span className="sftp-name" title={e.type === 'link' && e.target ? e.name + ' → ' + e.target : e.name}>
                          {e.name}
                          {e.type === 'link' && e.target ? <span className="sftp-link-target"> → {e.target}</span> : null}
                        </span>
                      )}
                    </span>
                  )
                }
                if (col === 'ext') {
                  return <span key={col} className="sftp-td sftp-ext">{fileExt(e.name, isDirEntry(e))}</span>
                }
                if (col === 'mode') {
                  return (
                    <button
                      key={col}
                      type="button"
                      className="sftp-td sftp-mode"
                      title="Права доступа"
                      onClick={(ev) => {
                        ev.stopPropagation()
                        openChmod(e)
                      }}
                    >
                      {fmtMode(e.mode)}
                    </button>
                  )
                }
                if (col === 'size') {
                  return <span key={col} className="sftp-td sftp-size">{fmtSize(e.size)}</span>
                }
                if (col === 'mtime') {
                  return <span key={col} className="sftp-td sftp-mtime">{fmtMtime(e.mtime)}</span>
                }
                return null
              })}
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
      {createPortal(
        <>
      {colMenu && (
        <AnchoredMenu className="sftp-col-menu" x={colMenu.x} y={colMenu.y}>
          {SFTP_COL_ORDER.map((id) => (
            <label key={id} className={'sftp-col-menu-item' + (id === 'name' ? ' locked' : '')}>
              <input
                type="checkbox"
                checked={colOn[id]}
                disabled={id === 'name'}
                onChange={() => toggleCol(id)}
              />
              {SFTP_COL_LABEL[id]}
            </label>
          ))}
        </AnchoredMenu>
      )}
      {ctxMenu && ctxMenu.pane === 'remote' && (
        <AnchoredMenu className="sftp-ctx-menu" x={ctxMenu.x} y={ctxMenu.y}>
          <CtxItem
            label={ctxRemoteItems.length > 1 ? 'Открыть (' + ctxRemoteItems.length + ')' : 'Открыть'}
            onPick={() => {
              closeMenus()
              openItems(ctxRemoteItems)
            }}
          />
          {ctxBuiltin.length > 0 && onOpenInEditor ? (
            <CtxItem
              label={ctxBuiltin.length > 1 ? 'Во встроенном редакторе (' + ctxBuiltin.length + ')' : 'Открыть во встроенном редакторе'}
              onPick={() => {
                closeMenus()
                openBuiltin(ctxBuiltin)
              }}
            />
          ) : null}
          {ctxExternal.length > 0 ? (
            <CtxItem
              label={ctxExternal.length > 1 ? 'Во внешнем редакторе (' + ctxExternal.length + ')' : 'Открыть во внешнем редакторе'}
              onPick={() => {
                closeMenus()
                void openExternalMany(ctxExternal)
              }}
            />
          ) : null}
          <CtxItem
            label={ctxRemoteItems.length > 1 ? 'Скачать (' + ctxRemoteItems.length + ')' : 'Скачать'}
            onPick={() => {
              closeMenus()
              void downloadMany(ctxRemoteItems)
            }}
          />
          {dualPane && ctxFiles.length > 0 ? (
            <CtxItem
              label="Скачать на этот компьютер"
              onPick={() => {
                closeMenus()
                void downloadToLocalMany(ctxFiles)
              }}
            />
          ) : null}
          <div className="sftp-ctx-sep" />
          {ctxRemoteItems.length === 1 ? (
            <CtxItem
              label="Переименовать"
              onPick={() => {
                closeMenus()
                startRename(ctxRemoteItems[0])
              }}
            />
          ) : null}
          <CtxItem
            label={ctxRemoteItems.length > 1 ? 'Удалить (' + ctxRemoteItems.length + ')' : 'Удалить'}
            danger
            onPick={() => {
              closeMenus()
              void removeMany(ctxRemoteItems)
            }}
          />
          <div className="sftp-ctx-sep" />
          <CtxItem
            label="Свойства"
            onPick={() => {
              closeMenus()
              setPropsOpen({ pane: 'remote', names: ctxMenu.names })
            }}
          />
        </AnchoredMenu>
      )}
      {ctxMenu && ctxMenu.pane === 'local' && (
        <AnchoredMenu className="sftp-ctx-menu" x={ctxMenu.x} y={ctxMenu.y}>
          {ctxLocalItems.length === 1 && ctxLocalItems[0].type === 'dir' ? (
            <CtxItem
              label="Открыть"
              onPick={() => {
                closeMenus()
                loadLocal(joinLocal(localPath, ctxLocalItems[0].name))
              }}
            />
          ) : null}
          {ctxLocalItems.some((e) => e.type !== 'dir') ? (
            <CtxItem
              label={ctxLocalItems.length > 1 ? 'Загрузить на сервер (' + ctxLocalItems.length + ')' : 'Загрузить на сервер'}
              onPick={() => {
                closeMenus()
                void uploadLocalMany(ctxLocalItems)
              }}
            />
          ) : null}
          <div className="sftp-ctx-sep" />
          <CtxItem
            label="Свойства"
            onPick={() => {
              closeMenus()
              setPropsOpen({ pane: 'local', names: ctxMenu.names })
            }}
          />
        </AnchoredMenu>
      )}
      {overwriteNames && (
        <OverwriteAsk names={overwriteNames} onYes={() => answerOverwrite(true)} onNo={() => answerOverwrite(false)} />
      )}
      {propsOpen && (
        <PropsSheet
          title={propsRowsFor(propsOpen.pane, propsOpen.names).title}
          rows={propsRowsFor(propsOpen.pane, propsOpen.names).rows}
          onClose={() => setPropsOpen(null)}
        />
      )}
        </>,
        document.body
      )}
    </div>
  )
}
