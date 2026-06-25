import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { MouseEvent as ReactMouseEvent } from 'react'
import type { KIPrompt, ServerConfig } from '../shared/types'
import { Sidebar } from './components/Sidebar'
import { TabBar } from './components/TabBar'
import { PaneView } from './components/PaneView'
import { ServerForm } from './components/ServerForm'
import { SftpPanel } from './components/SftpPanel'
import { SettingsModal } from './components/SettingsModal'
import { KiModal } from './components/KiModal'
import { KeyGenModal } from './components/KeyGenModal'
import { StatusBar } from './components/StatusBar'
import { CodeEditor } from './components/CodeEditor'
import { CommandPalette, type PaletteItem } from './components/CommandPalette'
import { useSettings } from './SettingsContext'
import { bindingLookup, comboFromEvent } from './keybindings'
import { applyUiTheme } from './themes'
import {
  type PaneNode,
  type PaneLeaf,
  makeLeaf,
  firstLeaf,
  findLeaf,
  allLeaves,
  updateLeaf,
  updateLeafBySession,
  splitLeaf,
  removeLeaf,
  updateSplitSizes,
  serializePane,
  deserializePane
} from './paneTree'

export interface Tab {
  key: string
  title: string
  /** Вкладка-терминал (дерево панелей) или вкладка-редактор файла. */
  kind: 'terminal' | 'editor'
  root: PaneNode
  activePaneId: string
  sftpOpen: boolean
  /** Для kind==='editor': какой файл и на какой сессии редактируется. */
  editor?: { sessionId: string; remotePath: string }
  editorDirty?: boolean
}

/** Что открыть в новой панели при сплите: локальный терминал или конкретный сервер. */
export type SplitChoice = { kind: 'local' } | { kind: 'ssh'; serverId: string; title: string }

function uid(): string {
  return crypto.randomUUID()
}

export default function App(): JSX.Element {
  const [servers, setServers] = useState<ServerConfig[]>([])
  const [tabs, setTabs] = useState<Tab[]>([])
  const [activeKey, setActiveKey] = useState<string | null>(null)
  const [editing, setEditing] = useState<ServerConfig | null | undefined>(undefined)
  const [showSettings, setShowSettings] = useState(false)
  const [showKeyGen, setShowKeyGen] = useState(false)
  const [broadcast, setBroadcast] = useState(false)
  const [sftpWidth, setSftpWidth] = useState(380)
  const [sftpClosing, setSftpClosing] = useState<Record<string, boolean>>({})
  const [sidebarWidth, setSidebarWidth] = useState(270)
  const [kiRequest, setKiRequest] = useState<{ id: string; prompts: KIPrompt[] } | null>(null)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const { settings, update } = useSettings()

  const tabsRef = useRef<Tab[]>([])
  tabsRef.current = tabs
  const activeKeyRef = useRef<string | null>(activeKey)
  activeKeyRef.current = activeKey
  const settingsRef = useRef(settings)
  settingsRef.current = settings
  const broadcastRef = useRef(broadcast)
  broadcastRef.current = broadcast

  // Подтягиваем сохранённые ширины панелей.
  useEffect(() => {
    if (settings.sidebarWidth) setSidebarWidth(settings.sidebarWidth)
    if (settings.sftpWidth) setSftpWidth(settings.sftpWidth)
  }, [settings.sidebarWidth, settings.sftpWidth])

  // Применяем цветовую схему ко всему интерфейсу (CSS-переменные), не только к терминалу.
  useEffect(() => {
    applyUiTheme(settings.theme)
  }, [settings.theme])

  // Плотность интерфейса (compact/comfortable) — через data-атрибут на :root.
  useEffect(() => {
    document.documentElement.dataset.density = settings.density ?? 'comfortable'
  }, [settings.density])

  const reloadServers = useCallback(async () => {
    setServers(await window.api.servers.list())
  }, [])

  useEffect(() => {
    reloadServers()
  }, [reloadServers])

  // Обновление статуса/завершения сессий — патчим соответствующий лист по sessionId.
  const reconnectPane = useCallback((tabKey: string, paneId: string) => {
    setTabs((prev) =>
      prev.map((t) =>
        t.key === tabKey
          ? {
              ...t,
              root: updateLeaf(t.root, paneId, (l) => ({
                gen: l.gen + 1,
                status: 'connecting',
                statusMsg: undefined,
                sessionId: undefined
              }))
            }
          : t
      )
    )
  }, [])

  useEffect(() => {
    return window.api.session.onKi((p) => setKiRequest(p))
  }, [])

  useEffect(() => {
    const off = window.api.session.onStatus((p) => {
      setTabs((prev) =>
        prev.map((t) => ({ ...t, root: updateLeafBySession(t.root, p.id, { status: p.status, statusMsg: p.message }) }))
      )
    })
    const offExit = window.api.session.onExit((p) => {
      let reconnect: { tabKey: string; paneId: string } | null = null
      setTabs((prev) =>
        prev.map((t) => {
          const leaf = allLeaves(t.root).find((l) => l.sessionId === p.id)
          if (!leaf) return t
          if (settingsRef.current.autoReconnect && leaf.kind === 'ssh' && leaf.status === 'connected') {
            reconnect = { tabKey: t.key, paneId: leaf.id }
          }
          return {
            ...t,
            root: updateLeafBySession(t.root, p.id, {
              status: 'closed',
              statusMsg: p.error ?? 'Сессия завершена',
              sessionId: undefined
            })
          }
        })
      )
      if (reconnect) setTimeout(() => reconnectPane(reconnect!.tabKey, reconnect!.paneId), 1500)
    })
    return () => {
      off()
      offExit()
    }
  }, [reconnectPane])

  const openServerTab = useCallback((server: ServerConfig) => {
    const leaf = makeLeaf('ssh', server.name, server.id)
    const key = uid()
    setTabs((prev) => [...prev, { key, title: server.name, kind: 'terminal', root: leaf, activePaneId: leaf.id, sftpOpen: false }])
    setActiveKey(key)
  }, [])

  const openLocalTab = useCallback(() => {
    const leaf = makeLeaf('local', 'Локальный терминал')
    const key = uid()
    setTabs((prev) => [...prev, { key, title: 'Локальный терминал', kind: 'terminal', root: leaf, activePaneId: leaf.id, sftpOpen: false }])
    setActiveKey(key)
  }, [])

  // Открыть встроенный редактор удалённого файла в отдельной вкладке.
  const openEditorTab = useCallback((sessionId: string, remotePath: string) => {
    const fileName = remotePath.split('/').pop() || remotePath
    const existing = tabsRef.current.find(
      (t) => t.kind === 'editor' && t.editor?.sessionId === sessionId && t.editor?.remotePath === remotePath
    )
    if (existing) {
      setActiveKey(existing.key)
      return
    }
    const leaf = makeLeaf('local', fileName) // placeholder-лист (вкладка-редактор не использует дерево)
    const key = uid()
    setTabs((prev) => [
      ...prev,
      { key, title: fileName, kind: 'editor', root: leaf, activePaneId: leaf.id, sftpOpen: false, editor: { sessionId, remotePath } }
    ])
    setActiveKey(key)
  }, [])

  const setEditorDirty = useCallback((key: string, dirty: boolean) => {
    setTabs((prev) => prev.map((t) => (t.key === key ? { ...t, editorDirty: dirty } : t)))
  }, [])

  // Разделить активную панель: в новой панели открываем выбранный сервер или локальный терминал.
  const splitPane = useCallback((tabKey: string, dir: 'row' | 'col', choice: SplitChoice) => {
    setTabs((prev) =>
      prev.map((t) => {
        if (t.key !== tabKey) return t
        const cur = findLeaf(t.root, t.activePaneId)
        if (!cur) return t
        const fresh =
          choice.kind === 'local'
            ? makeLeaf('local', 'Локальный терминал')
            : makeLeaf('ssh', choice.title, choice.serverId)
        return { ...t, root: splitLeaf(t.root, cur.id, dir, fresh), activePaneId: fresh.id }
      })
    )
  }, [])

  const focusPane = useCallback((tabKey: string, paneId: string) => {
    setTabs((prev) => prev.map((t) => (t.key === tabKey ? { ...t, activePaneId: paneId } : t)))
  }, [])

  const closePane = useCallback((tabKey: string, paneId: string) => {
    const tab = tabsRef.current.find((t) => t.key === tabKey)
    const leaf = tab && findLeaf(tab.root, paneId)
    if (leaf?.sessionId) window.api.session.close(leaf.sessionId)
    setTabs((prev) => {
      const next: Tab[] = []
      for (const t of prev) {
        if (t.key !== tabKey) {
          next.push(t)
          continue
        }
        const root = removeLeaf(t.root, paneId)
        if (!root) continue // последняя панель — вкладка закрывается
        const stillActive = findLeaf(root, t.activePaneId)
        next.push({ ...t, root, activePaneId: stillActive ? t.activePaneId : firstLeaf(root).id })
      }
      setActiveKey((cur) => (next.some((t) => t.key === cur) ? cur : next.length ? next[next.length - 1].key : null))
      return next
    })
  }, [])

  const resizeSplit = useCallback((tabKey: string, splitId: string, sizes: [number, number]) => {
    setTabs((prev) => prev.map((t) => (t.key === tabKey ? { ...t, root: updateSplitSizes(t.root, splitId, sizes) } : t)))
  }, [])

  const handleReady = useCallback((paneId: string, sessionId: string) => {
    setTabs((prev) => prev.map((t) => ({ ...t, root: updateLeaf(t.root, paneId, { sessionId }) })))
  }, [])

  // Broadcast ограничен панелями ТЕКУЩЕЙ вкладки (а не всеми вкладками) —
  // чтобы случайно не отправить команду в прод-сессию из другой вкладки.
  const broadcastInput = useCallback((fromId: string, data: string) => {
    if (!broadcastRef.current) return
    const tab = tabsRef.current.find((t) => allLeaves(t.root).some((l) => l.sessionId === fromId))
    if (!tab) return
    for (const l of allLeaves(tab.root)) {
      if (l.sessionId && l.sessionId !== fromId) window.api.session.write(l.sessionId, data)
    }
  }, [])

  // Сколько сессий получит broadcast-ввод (для индикатора): панели активной вкладки.
  const broadcastTargets = useMemo(() => {
    const tab = tabs.find((t) => t.key === activeKey)
    if (!tab) return 0
    return allLeaves(tab.root).filter((l) => l.sessionId).length
  }, [tabs, activeKey])

  // Живой статус подключения по серверу (агрегируем по всем вкладкам/панелям).
  const serverStatuses = useMemo(() => {
    const rank: Record<string, number> = { connected: 3, connecting: 2, error: 1 }
    const out: Record<string, 'connected' | 'connecting' | 'error'> = {}
    for (const t of tabs) {
      for (const l of allLeaves(t.root)) {
        if (l.kind !== 'ssh' || !l.serverId) continue
        const st = l.status === 'connected' ? 'connected' : l.status === 'error' ? 'error' : l.status === 'connecting' ? 'connecting' : null
        if (!st) continue
        if (!out[l.serverId] || rank[st] > rank[out[l.serverId]]) out[l.serverId] = st
      }
    }
    return out
  }, [tabs])

  const closeTab = useCallback((key: string) => {
    const tab = tabsRef.current.find((t) => t.key === key)
    if (tab?.kind === 'editor' && tab.editorDirty) {
      if (!confirm(`В «${tab.title}» есть несохранённые изменения. Закрыть без сохранения?`)) return
    }
    if (tab) for (const l of allLeaves(tab.root)) if (l.sessionId) window.api.session.close(l.sessionId)
    setTabs((prev) => {
      const next = prev.filter((t) => t.key !== key)
      setActiveKey((cur) => (cur !== key ? cur : next.length ? next[next.length - 1].key : null))
      return next
    })
  }, [])

  const renameTab = useCallback((key: string, title: string) => {
    setTabs((prev) => prev.map((t) => (t.key === key ? { ...t, title } : t)))
  }, [])

  const reorderTabs = useCallback((fromKey: string, toKey: string) => {
    setTabs((prev) => {
      const from = prev.findIndex((t) => t.key === fromKey)
      const to = prev.findIndex((t) => t.key === toKey)
      if (from < 0 || to < 0 || from === to) return prev
      const next = [...prev]
      const [moved] = next.splice(from, 1)
      next.splice(to, 0, moved)
      return next
    })
  }, [])

  const toggleSftp = useCallback((key: string) => {
    const tab = tabsRef.current.find((t) => t.key === key)
    if (tab?.sftpOpen) {
      setSftpClosing((s) => ({ ...s, [key]: true }))
      setTimeout(() => {
        setSftpClosing((s) => {
          const n = { ...s }
          delete n[key]
          return n
        })
        setTabs((prev) => prev.map((t) => (t.key === key ? { ...t, sftpOpen: false } : t)))
      }, 200)
    } else {
      setTabs((prev) => prev.map((t) => (t.key === key ? { ...t, sftpOpen: true } : t)))
    }
  }, [])

  const startSftpResize = useCallback(
    (e: ReactMouseEvent) => {
      e.preventDefault()
      const onMove = (ev: MouseEvent): void => setSftpWidth(Math.max(260, Math.min(820, window.innerWidth - ev.clientX)))
      const onUp = (): void => {
        document.removeEventListener('mousemove', onMove)
        document.removeEventListener('mouseup', onUp)
        document.body.style.cursor = ''
        setSftpWidth((w) => {
          update({ sftpWidth: w })
          return w
        })
      }
      document.body.style.cursor = 'col-resize'
      document.addEventListener('mousemove', onMove)
      document.addEventListener('mouseup', onUp)
    },
    [update]
  )

  const startSidebarResize = useCallback(
    (e: ReactMouseEvent) => {
      e.preventDefault()
      const onMove = (ev: MouseEvent): void => setSidebarWidth(Math.max(190, Math.min(520, ev.clientX)))
      const onUp = (): void => {
        document.removeEventListener('mousemove', onMove)
        document.removeEventListener('mouseup', onUp)
        document.body.style.cursor = ''
        setSidebarWidth((w) => {
          update({ sidebarWidth: w })
          return w
        })
      }
      document.body.style.cursor = 'col-resize'
      document.addEventListener('mousemove', onMove)
      document.addEventListener('mouseup', onUp)
    },
    [update]
  )

  // Запуск: восстановление вкладок (если включено) или локальный терминал.
  const startedOnceRef = useRef(false)
  useEffect(() => {
    if (startedOnceRef.current) return
    startedOnceRef.current = true
    void (async () => {
      const s = await window.api.settings.get()
      if (s.restoreTabsOnStart) {
        const saved = await window.api.layout.get()
        if (saved.length) {
          const restored: Tab[] = saved.map((st) => {
            const root = deserializePane(st.root)
            return { key: uid(), title: st.title, kind: 'terminal', root, activePaneId: firstLeaf(root).id, sftpOpen: false }
          })
          setTabs(restored)
          setActiveKey(restored[restored.length - 1].key)
          return
        }
      }
      if (s.openLocalOnStart) openLocalTab()
    })()
  }, [openLocalTab])

  // Сохранение раскладки терминальных вкладок (с дебаунсом) для восстановления.
  useEffect(() => {
    if (!startedOnceRef.current) return
    const id = setTimeout(() => {
      const payload = tabs
        .filter((t) => t.kind === 'terminal')
        .map((t) => ({ title: t.title, root: serializePane(t.root) }))
      void window.api.layout.set(payload)
    }, 600)
    return () => clearTimeout(id)
  }, [tabs])

  // Глобальные горячие клавиши. Capture-фаза — чтобы перехватывать до xterm.
  useEffect(() => {
    const lookup = bindingLookup(settingsRef.current)
    const cycleTab = (dir: 1 | -1): void => {
      const list = tabsRef.current
      if (list.length < 2) return
      const idx = list.findIndex((t) => t.key === activeKeyRef.current)
      const next = list[(idx + dir + list.length) % list.length]
      setActiveKey(next.key)
    }
    const cyclePane = (dir: 1 | -1): void => {
      const key = activeKeyRef.current
      if (!key) return
      const tab = tabsRef.current.find((t) => t.key === key)
      if (!tab || tab.kind !== 'terminal') return
      const leaves = allLeaves(tab.root)
      if (leaves.length < 2) return
      const idx = leaves.findIndex((l) => l.id === tab.activePaneId)
      const next = leaves[(idx + dir + leaves.length) % leaves.length]
      focusPane(key, next.id)
    }
    const onKey = (e: KeyboardEvent): void => {
      // Не мешаем записи новой комбинации в настройках.
      if ((document.activeElement as HTMLElement | null)?.hasAttribute('data-keycapture')) return
      const combo = comboFromEvent(e)
      if (!combo) return
      const action = lookup.get(combo)
      if (!action) return
      const key = activeKeyRef.current
      const tab = tabsRef.current.find((t) => t.key === key)
      const isTerm = tab?.kind === 'terminal'
      let handled = true
      switch (action) {
        case 'command-palette': setPaletteOpen((v) => !v); break
        case 'new-terminal': openLocalTab(); break
        case 'close-tab': if (key) closeTab(key); break
        case 'next-tab': cycleTab(1); break
        case 'prev-tab': cycleTab(-1); break
        case 'split-right': if (key && isTerm) splitPane(key, 'row', { kind: 'local' }); break
        case 'split-down': if (key && isTerm) splitPane(key, 'col', { kind: 'local' }); break
        case 'close-pane': if (key && isTerm && tab) closePane(key, tab.activePaneId); break
        case 'focus-next-pane': cyclePane(1); break
        case 'focus-prev-pane': cyclePane(-1); break
        case 'toggle-sftp': if (key && isTerm) toggleSftp(key); break
        case 'toggle-broadcast': setBroadcast((b) => !b); break
        case 'open-settings': setShowSettings(true); break
        default: handled = false
      }
      if (handled) {
        e.preventDefault()
        e.stopImmediatePropagation()
      }
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [settings.keybindings, openLocalTab, closeTab, splitPane, closePane, toggleSftp])

  const saveServer = useCallback(
    async (cfg: ServerConfig) => {
      await window.api.servers.save(cfg)
      await reloadServers()
      setEditing(undefined)
    },
    [reloadServers]
  )

  const deleteServer = useCallback(
    async (id: string) => {
      await window.api.servers.remove(id)
      await reloadServers()
    },
    [reloadServers]
  )

  const importServers = useCallback(
    async (kind: 'ssh' | 'putty') => {
      try {
        const r = kind === 'ssh'
          ? await window.api.servers.importSshConfig()
          : await window.api.servers.importPutty()
        await reloadServers()
        alert(`Импортировано серверов: ${r.imported}`)
      } catch (e) {
        alert('Ошибка импорта: ' + (e as Error).message)
      }
    },
    [reloadServers]
  )

  const paletteItems = useMemo<PaletteItem[]>(() => {
    const items: PaletteItem[] = []
    for (const s of servers) {
      items.push({
        id: 'srv:' + s.id,
        label: s.name,
        hint: `${s.username}@${s.host}`,
        icon: '🔌',
        group: 'Сервер',
        run: () => openServerTab(s)
      })
    }
    items.push({ id: 'act:local', label: 'Новый локальный терминал', icon: '🖥', group: 'Действие', run: openLocalTab })
    items.push({ id: 'act:settings', label: 'Настройки', icon: '⚙', group: 'Действие', run: () => setShowSettings(true) })
    items.push({ id: 'act:keygen', label: 'Генерация ключей', icon: '🔑', group: 'Действие', run: () => setShowKeyGen(true) })
    items.push({ id: 'act:newserver', label: 'Добавить сервер', icon: '➕', group: 'Действие', run: () => setEditing(null) })
    for (const t of tabs) {
      items.push({
        id: 'tab:' + t.key,
        label: t.title,
        hint: 'перейти к вкладке',
        icon: t.kind === 'editor' ? '📝' : '🗔',
        group: 'Вкладка',
        run: () => setActiveKey(t.key)
      })
    }
    return items
  }, [servers, tabs, openServerTab, openLocalTab])

  return (
    <div className="app">
      <Sidebar
        servers={servers}
        onConnect={openServerTab}
        onOpenLocal={openLocalTab}
        onNew={() => setEditing(null)}
        onEdit={(s) => setEditing(s)}
        onDelete={deleteServer}
        onOpenSettings={() => setShowSettings(true)}
        onOpenKeyGen={() => setShowKeyGen(true)}
        onImport={importServers}
        width={sidebarWidth}
        statuses={serverStatuses}
      />

      <div className="sidebar-resizer" onMouseDown={startSidebarResize} />

      <div className="workspace">
        <TabBar
          tabs={tabs}
          activeKey={activeKey}
          servers={servers}
          onSelect={setActiveKey}
          onClose={closeTab}
          onNewLocal={openLocalTab}
          onToggleSftp={toggleSftp}
          onRename={renameTab}
          onReorder={reorderTabs}
          onSplit={splitPane}
          broadcast={broadcast}
          onToggleBroadcast={() => setBroadcast((b) => !b)}
          onEditServer={(s) => setEditing(s)}
        />

        <div className="terminals">
          {tabs.length === 0 && (
            <div className="empty-state">
              <h1>TermiNAL</h1>
              <p>Выберите сервер слева для подключения по SSH<br />или откройте локальный терминал.</p>
              <button onClick={openLocalTab}>Открыть локальный терминал</button>
            </div>
          )}

          {tabs.map((tab) => {
            const isActive = tab.key === activeKey
            if (tab.kind === 'editor' && tab.editor) {
              return (
                <div key={tab.key} className="terminal-slot" style={{ display: isActive ? 'flex' : 'none' }}>
                  <CodeEditor
                    sessionId={tab.editor.sessionId}
                    remotePath={tab.editor.remotePath}
                    fileName={tab.title}
                    active={isActive}
                    onDirtyChange={(d) => setEditorDirty(tab.key, d)}
                  />
                </div>
              )
            }
            const leaf = findLeaf(tab.root, tab.activePaneId)
            // SFTP доступен только по реально установленному соединению, иначе клиента ещё/уже нет.
            const sftpSession = leaf && leaf.kind === 'ssh' && leaf.status === 'connected' ? leaf.sessionId : undefined
            return (
              <div
                key={tab.key}
                className="terminal-slot"
                style={{ display: isActive ? 'flex' : 'none' }}
              >
                <div className="pane-area">
                  <PaneView
                    node={tab.root}
                    activePaneId={tab.activePaneId}
                    tabActive={tab.key === activeKey}
                    canClose={allLeaves(tab.root).length > 1}
                    onFocusPane={(pid) => focusPane(tab.key, pid)}
                    onReady={handleReady}
                    onInput={broadcastInput}
                    onClosePane={(pid) => closePane(tab.key, pid)}
                    onReconnect={(pid) => reconnectPane(tab.key, pid)}
                    onResizeSplit={(sid, sizes) => resizeSplit(tab.key, sid, sizes)}
                  />
                </div>
                {tab.sftpOpen && sftpSession && (
                  <>
                    <div className="sftp-resizer" onMouseDown={startSftpResize} />
                    <SftpPanel
                      sessionId={sftpSession}
                      width={sftpWidth}
                      closing={!!sftpClosing[tab.key]}
                      onClose={() => toggleSftp(tab.key)}
                      onOpenInEditor={(rp) => openEditorTab(sftpSession, rp)}
                    />
                  </>
                )}
              </div>
            )
          })}
        </div>

        {(() => {
          const activeTab = tabs.find((t) => t.key === activeKey)
          if (activeTab?.kind === 'editor' && activeTab.editor) {
            return (
              <StatusBar
                leaf={undefined}
                server={undefined}
                broadcast={false}
                editor={{ remotePath: activeTab.editor.remotePath, dirty: !!activeTab.editorDirty }}
              />
            )
          }
          const activeLeaf = activeTab ? findLeaf(activeTab.root, activeTab.activePaneId) : undefined
          const srv = servers.find((s) => s.id === activeLeaf?.serverId)
          return <StatusBar leaf={activeLeaf} server={srv} broadcast={broadcast} broadcastTargets={broadcastTargets} />
        })()}
      </div>

      {editing !== undefined && (
        <ServerForm initial={editing} servers={servers} onCancel={() => setEditing(undefined)} onSave={saveServer} />
      )}

      {paletteOpen && <CommandPalette items={paletteItems} onClose={() => setPaletteOpen(false)} />}

      {showSettings && <SettingsModal onClose={() => setShowSettings(false)} />}

      {showKeyGen && (
        <KeyGenModal
          connectedSessions={tabs.flatMap((t) =>
            allLeaves(t.root)
              .filter((l) => l.kind === 'ssh' && l.status === 'connected' && l.sessionId)
              .map((l) => ({ sessionId: l.sessionId!, title: `${t.title} — ${l.title}` }))
          )}
          onClose={() => setShowKeyGen(false)}
        />
      )}

      {kiRequest && (
        <KiModal
          sessionId={kiRequest.id}
          prompts={kiRequest.prompts}
          onSubmit={(answers) => {
            void window.api.session.respondKi(kiRequest.id, answers)
            setKiRequest(null)
          }}
          onCancel={() => {
            void window.api.session.respondKi(kiRequest.id, [])
            setKiRequest(null)
          }}
        />
      )}
    </div>
  )
}

export type { PaneLeaf }
