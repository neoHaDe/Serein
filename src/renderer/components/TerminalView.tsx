import { useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { SearchAddon } from '@xterm/addon-search'
import { useSettings } from '../SettingsContext'
import { getTheme } from '../themes'

interface Props {
  paneId: string
  /** Стабильный ключ инстанса (paneId:gen) — переживает split, меняется при reconnect. */
  instanceKey: string
  kind: 'ssh' | 'local'
  serverId?: string
  active: boolean
  focused: boolean
  onReady: (paneId: string, sessionId: string) => void
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
}

const registry = new Map<string, PaneTerm>()

export function TerminalView({ paneId, instanceKey, kind, serverId, active, focused, onReady, onInput }: Props): JSX.Element {
  const mountRef = useRef<HTMLDivElement>(null)
  const entryRef = useRef<PaneTerm | null>(null)

  const { settings, update } = useSettings()
  const settingsRef = useRef(settings)
  settingsRef.current = settings

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
      term.open(host)

      const created: PaneTerm = { host, term, fit, search, sessionId: null, offData: () => {}, onInput, detached: false }

      term.attachCustomKeyEventHandler((e) => {
        if (e.type !== 'keydown') return true
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

      created.offData = window.api.session.onData((p) => {
        if (p.id === created.sessionId) term.write(p.data)
      })
      term.onData((d) => {
        if (created.sessionId) {
          window.api.session.write(created.sessionId, d)
          created.onInput?.(created.sessionId, d)
        }
      })

      registry.set(instanceKey, created)
      entry = created

      // Открытие сессии — единожды на инстанс.
      const { cols, rows } = term
      const openPromise =
        kind === 'ssh' && serverId
          ? window.api.session.openSsh({ serverId, cols, rows })
          : window.api.session.openLocal({ cols, rows })
      openPromise
        .then((id) => {
          created.sessionId = id
          onReady(paneId, id)
        })
        .catch((err: Error) => {
          term.writeln(`\r\n\x1b[31mОшибка подключения: ${err.message}\x1b[0m`)
        })
    }

    entry.detached = false
    entryRef.current = entry
    mount.appendChild(entry.host)

    const fitNow = (): void => {
      try {
        entry!.fit.fit()
        if (entry!.sessionId)
          window.api.session.resize({ id: entry!.sessionId, cols: entry!.term.cols, rows: entry!.term.rows })
      } catch {
        /* контейнер мог быть скрыт */
      }
    }
    requestAnimationFrame(fitNow)

    const ro = new ResizeObserver(() => {
      if (active) fitNow()
    })
    ro.observe(mount)

    return () => {
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
            cur.offData()
            cur.term.dispose()
            registry.delete(instanceKey)
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
      try {
        e.fit.fit()
        if (e.sessionId) window.api.session.resize({ id: e.sessionId, cols: e.term.cols, rows: e.term.rows })
      } catch {
        /* ignore */
      }
    })
  }, [settings.theme, settings.fontSize, settings.fontFamily])

  // Фокус/размер при показе панели.
  useEffect(() => {
    if (!active) return
    const e = entryRef.current
    if (!e) return
    requestAnimationFrame(() => {
      try {
        e.fit.fit()
        if (focused) e.term.focus()
        if (e.sessionId) window.api.session.resize({ id: e.sessionId, cols: e.term.cols, rows: e.term.rows })
      } catch {
        /* ignore */
      }
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
