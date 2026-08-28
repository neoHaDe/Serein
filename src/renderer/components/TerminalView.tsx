import { useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { SearchAddon } from '@xterm/addon-search'
import { useSettings } from '../SettingsContext'
import { getTheme } from '../themes'
import { shouldPreserveSession } from '../detachedSessions'

interface Props {
  paneId: string
  /** Стабильный ключ инстанса (paneId:gen) — переживает split, меняется при reconnect. */
  instanceKey: string
  kind: 'ssh' | 'local'
  serverId?: string
  /** Подключиться к уже открытой сессии (откреплённая вкладка). */
  attachSessionId?: string
  active: boolean
  focused: boolean
  onReady: (paneId: string, sessionId: string) => void
  onFail?: (paneId: string, message: string) => void
  onInput?: (fromSessionId: string, data: string) => void
}

/**
 * Живой терминал, привязанный к инстансу (paneId:gen), а НЕ к монтированию компонента.
 * При split дерево панелей перестраивается и React перемонтирует компонент — но xterm и
 * SSH/PTY-сессия сохраняются в этом реестре, поэтому уже открытая панель не сбрасывается.
 */
interface PaneTerm {
  host: HTMLDivElement
  term: Terminal
  fit: FitAddon
  search: SearchAddon
  sessionId: string | null
  offData: () => void
  onInput?: (fromSessionId: string, data: string) => void
  detached: boolean
  opened: boolean
  sessionStarted: boolean
  lastCols: number
  lastRows: number
}

const registry = new Map<string, PaneTerm>()

function copyText(text: string): void {
  if (!text) return
  void window.api.clipboard.write(text)
}

function pasteInto(term: Terminal): void {
  void window.api.clipboard.read().then((t) => {
    if (t) term.paste(t)
  })
}

/** Правый клик не должен сбрасывать выделение; копирование — через Win32, не navigator.clipboard. */
function bindTermClipboard(term: Terminal, host: HTMLDivElement): void {
  host.addEventListener(
    'mousedown',
    (e) => {
      if (e.button === 2) {
        e.preventDefault()
        e.stopImmediatePropagation()
      }
    },
    true
  )
  host.addEventListener('contextmenu', (e) => {
    e.preventDefault()
    e.stopPropagation()
    const sel = term.getSelection()
    if (sel) copyText(sel)
    else pasteInto(term)
  })
  term.parser.registerOscHandler(52, (data) => {
    const i = data.indexOf(';')
    if (i < 0) return true
    const payload = data.slice(i + 1)
    if (!payload || payload === '?') return true
    try {
      const bin = atob(payload)
      const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0))
      copyText(new TextDecoder().decode(bytes))
    } catch {
      /* bad OSC 52 */
    }
    return true
  })
}

const dataWriters = new Map<string, (data: string) => void>()
let dataBusOff: (() => void) | undefined

function dataBusStart(): void {
  if (dataBusOff) return
  dataBusOff = window.api.session.onData((p) => {
    dataWriters.get(p.id)?.(p.data)
  })
}

function bindWriter(id: string, write: (data: string) => void): () => void {
  dataBusStart()
  let buf = ''
  let raf = 0
  const flush = (): void => {
    raf = 0
    if (!buf) return
    const s = buf
    buf = ''
    write(s)
  }
  dataWriters.set(id, (data) => {
    buf += data
    if (buf.length >= 64 * 1024) {
      if (raf) cancelAnimationFrame(raf)
      flush()
      return
    }
    if (!raf) raf = requestAnimationFrame(flush)
  })
  return () => {
    if (raf) cancelAnimationFrame(raf)
    flush()
    dataWriters.delete(id)
  }
}

function visibleEnough(el: HTMLElement): boolean {
  if (!el.isConnected) return false
  if (el.clientWidth < 48 || el.clientHeight < 48) return false
  const st = getComputedStyle(el)
  return st.display !== 'none' && st.visibility !== 'hidden'
}

/** Fit только при реальном размере; крошечный PTY ломает TUI и «сжимает» вывод. */
function applyFit(entry: PaneTerm, notifyPty: boolean): boolean {
  if (!visibleEnough(entry.host)) return false
  const dim = entry.fit.proposeDimensions()
  if (!dim || dim.cols < 20 || dim.rows < 5) return false
  if (dim.cols !== entry.term.cols || dim.rows !== entry.term.rows) {
    try {
      entry.fit.fit()
    } catch {
      return false
    }
  }
  const cols = entry.term.cols
  const rows = entry.term.rows
  if (cols === entry.lastCols && rows === entry.lastRows) return true
  entry.lastCols = cols
  entry.lastRows = rows
  if (notifyPty && entry.sessionId) {
    window.api.session.resize({ id: entry.sessionId, cols, rows })
  }
  return true
}

export function TerminalView({
  paneId,
  instanceKey,
  kind,
  serverId,
  attachSessionId,
  active,
  focused,
  onReady,
  onFail,
  onInput
}: Props): JSX.Element {
  const mountRef = useRef<HTMLDivElement>(null)
  const entryRef = useRef<PaneTerm | null>(null)
  const attachRef = useRef(attachSessionId)
  attachRef.current = attachSessionId

  const { settings, update } = useSettings()
  const settingsRef = useRef(settings)
  settingsRef.current = settings
  const onFailRef = useRef(onFail)
  onFailRef.current = onFail

  const [searchOpen, setSearchOpen] = useState(false)
  const [searchTerm, setSearchTerm] = useState('')
  const searchInputRef = useRef<HTMLInputElement>(null)

  // Держим актуальный onInput в реестре (после перемонтирования он новый).
  useEffect(() => {
    if (entryRef.current) entryRef.current.onInput = onInput
  })

  useEffect(() => {
    if (!mountRef.current) return
    const mount = mountRef.current

    let entry = registry.get(instanceKey)
    if (!entry) {
      // Первое создание этого инстанса: xterm + аддоны + сессия.
      const s = settingsRef.current
      const host = document.createElement('div')
      host.className = 'terminal-host'
      const term = new Terminal({
        fontFamily: s.fontFamily,
        fontSize: s.fontSize,
        cursorBlink: true,
        scrollback: 10000,
        theme: getTheme(s.theme)
      })
      const fit = new FitAddon()
      const search = new SearchAddon()
      term.loadAddon(fit)
      term.loadAddon(search)
      term.loadAddon(new WebLinksAddon())
      bindTermClipboard(term, host)

      const created: PaneTerm = {
        host,
        term,
        fit,
        search,
        sessionId: null,
        offData: () => {},
        onInput,
        detached: false,
        opened: false,
        sessionStarted: false,
        lastCols: 0,
        lastRows: 0
      }

      term.attachCustomKeyEventHandler((e) => {
        if (e.type !== 'keydown') return true
        if (e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey && e.code === 'KeyC') {
          e.preventDefault()
          e.stopPropagation()
          const text = term.getSelection()
          if (text) copyText(text)
          return false
        }
        if (e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey && e.code === 'KeyV') {
          e.preventDefault()
          e.stopPropagation()
          pasteInto(term)
          return false
        }
        if (e.ctrlKey && (e.key === 'f' || e.key === 'F')) {
          setSearchOpen(true)
          requestAnimationFrame(() => searchInputRef.current?.focus())
          return false
        }
        if (e.ctrlKey && (e.key === '+' || e.key === '=')) {
          update({ fontSize: Math.min(32, settingsRef.current.fontSize + 1) })
          return false
        }
        if (e.ctrlKey && e.key === '-') {
          update({ fontSize: Math.max(8, settingsRef.current.fontSize - 1) })
          return false
        }
        if (e.ctrlKey && e.key === '0') {
          update({ fontSize: 14 })
          return false
        }
        if (e.key === 'Escape') {
          setSearchOpen(false)
          search.clearDecorations()
        }
        return true
      })

      term.onData((d) => {
        if (created.sessionId) {
          window.api.session.write(created.sessionId, d)
          created.onInput?.(created.sessionId, d)
        }
      })

      registry.set(instanceKey, created)
      entry = created
    }

    entry.detached = false
    entryRef.current = entry
    if (entry.host.parentElement !== mount) mount.appendChild(entry.host)
    if (!entry.opened) {
      entry.term.open(entry.host)
      entry.opened = true
    }

    const startSession = (e: PaneTerm): void => {
      const attach = attachRef.current
      if (attach) {
        if (e.sessionStarted && e.sessionId === attach) return
        e.sessionStarted = true
        e.sessionId = attach
        e.offData = bindWriter(attach, (data) => e.term.write(data))
        onReady(paneId, attach)
        return
      }
      if (e.sessionStarted) return
      e.sessionStarted = true
      const cols = e.lastCols >= 20 ? e.term.cols : 80
      const rows = e.lastRows >= 5 ? e.term.rows : 24
      const openPromise =
        kind === 'ssh' && serverId
          ? window.api.session.openSsh({ serverId, cols, rows })
          : window.api.session.openLocal({ cols, rows })
      openPromise
        .then((id) => {
          e.sessionId = id
          e.lastCols = e.term.cols
          e.lastRows = e.term.rows
          e.offData = bindWriter(id, (data) => e.term.write(data))
          onReady(paneId, id)
        })
        .catch((err: Error) => {
          e.term.writeln(`\r\n\x1b[31mОшибка подключения: ${err.message}\x1b[0m`)
          onFailRef.current?.(paneId, err.message)
        })
    }

    const fitNow = (): void => {
      if (applyFit(entry!, !!entry!.sessionId) && !entry!.sessionStarted) startSession(entry!)
    }

    let tries = 0
    const waitFit = (): void => {
      fitNow()
      if (!entry!.sessionStarted && tries++ < 45) requestAnimationFrame(waitFit)
      else if (!entry!.sessionStarted) startSession(entry!)
    }
    requestAnimationFrame(waitFit)

    let roTimer: number | undefined
    const ro = new ResizeObserver(() => {
      if (!active) return
      if (roTimer !== undefined) window.clearTimeout(roTimer)
      roTimer = window.setTimeout(() => {
        roTimer = undefined
        fitNow()
      }, 80)
    })
    ro.observe(mount)

    return () => {
      if (roTimer !== undefined) window.clearTimeout(roTimer)
      ro.disconnect()
      const e = registry.get(instanceKey)
      if (e && e.host.parentElement === mount) mount.removeChild(e.host)
      if (e) {
        e.detached = true
        // Если в этом же commit'е панель перемонтируется (split) — detached снова станет false
        // и инстанс сохранится. Если это реальное закрытие — освобождаем xterm.
        setTimeout(() => {
          const cur = registry.get(instanceKey)
          if (cur && cur.detached && !cur.host.isConnected) {
            const sid = cur.sessionId
            cur.offData()
            cur.term.dispose()
            registry.delete(instanceKey)
            if (sid && !shouldPreserveSession(sid)) void window.api.session.close(sid)
          }
        }, 120)
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Тема/шрифт.
  useEffect(() => {
    const e = entryRef.current
    if (!e) return
    e.term.options.theme = getTheme(settings.theme)
    e.term.options.fontSize = settings.fontSize
    e.term.options.fontFamily = settings.fontFamily
    requestAnimationFrame(() => {
      applyFit(e, true)
    })
  }, [settings.theme, settings.fontSize, settings.fontFamily])

  // Фокус/размер при показе панели.
  useEffect(() => {
    if (!active) return
    const e = entryRef.current
    if (!e) return
    requestAnimationFrame(() => {
      applyFit(e, true)
      if (focused) e.term.focus()
    })
  }, [active, focused])

  const doSearch = (next: boolean): void => {
    const search = entryRef.current?.search
    if (!search || !searchTerm) return
    const opts = { decorations: { matchOverviewRuler: '#e0af68', activeMatchColorOverviewRuler: '#f7768e' } }
    if (next) search.findNext(searchTerm, opts)
    else search.findPrevious(searchTerm, opts)
  }

  return (
    <div className="terminal-wrap">
      {searchOpen && (
        <div className="term-search">
          <input
            ref={searchInputRef}
            value={searchTerm}
            placeholder="Поиск…"
            onChange={(e) => {
              setSearchTerm(e.target.value)
              requestAnimationFrame(() => doSearch(true))
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') doSearch(!e.shiftKey)
              if (e.key === 'Escape') {
                setSearchOpen(false)
                entryRef.current?.search.clearDecorations()
                entryRef.current?.term.focus()
              }
            }}
          />
          <button className="mini" title="Назад (Shift+Enter)" onClick={() => doSearch(false)}>
            ↑
          </button>
          <button className="mini" title="Вперёд (Enter)" onClick={() => doSearch(true)}>
            ↓
          </button>
          <button
            className="mini"
            title="Закрыть (Esc)"
            onClick={() => {
              setSearchOpen(false)
              entryRef.current?.search.clearDecorations()
              entryRef.current?.term.focus()
            }}
          >
            ✕
          </button>
        </div>
      )}
      <div className="terminal-mount" ref={mountRef} />
    </div>
  )
}
