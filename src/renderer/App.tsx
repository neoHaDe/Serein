import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { MouseEvent as ReactMouseEvent } from 'react'
import type { SavedAuxWindow, ServerConfig, WorkspaceTool } from '../shared/types'
import { paneKindOf } from '../shared/types'
import type { Tab, SplitChoice } from './tabs'
import {
  broadcastTargets,
  closePaneIn,
  findTabKeyBySession,
  planReattach,
  planRestore,
  uid
} from './tabs'
import { shouldAutoReconnect } from './sessionRules'
import { Sidebar } from './components/Sidebar'
import { TabBar } from './components/TabBar'
import { PaneView } from './components/PaneView'
import { useReconnect } from './hooks/useReconnect'
import { useServerPrompts } from './hooks/useServerPrompts'
import { useAuxRestore } from './hooks/useAuxRestore'
import { ServerWorkspace } from './components/ServerWorkspace'
import { ServerForm } from './components/ServerForm'
import { SettingsModal } from './components/SettingsModal'
import { KiModal } from './components/KiModal'
import { HostKeyModal } from './components/HostKeyModal'
import { KeyGenModal } from './components/KeyGenModal'
import { GroupsModal } from './components/GroupsModal'
import { StatusBar } from './components/StatusBar'
import { CodeEditor } from './components/CodeEditor'
import { CommandPalette, type PaletteItem } from './components/CommandPalette'
import { detachedWindowLabel, openDetachedTabWindow } from './components/DetachedTabWindow'
import { handOverSession, takeOverSession } from './detachedSessions'
import type { ReattachSftpPayload, ReattachTabPayload, ReattachWorkspacePayload } from './reattach'
import { useSettings } from './SettingsContext'
import { bindingLookup, comboFromEvent } from './keybindings'
import { applyUiTheme } from './themes'
import { useWindowSnap } from './windowSnap'
import { isWindowsPlatform } from './platform'
import { MultiExecModal } from './components/MultiExecModal'
import {
  flushAuxPersist,
  listenAuxGeoEvents,
  markAuxPersistReady,
  seedAuxLive,
  setAuxPersistEnabled
} from './auxLayout'
import {
  type PaneLeaf,
  makeLeaf,
  findLeaf,
  allLeaves,
  updateLeaf,
  updateLeafBySession,
  splitLeaf,
  updateSplitSizes,
  serializePane
} from './paneTree'
import { errText } from './errText'

/** SSH-лист для рельсы: активная панель, иначе первый подключённый SSH. */
function sshLeafForTools(tab: Tab): PaneLeaf | undefined {
  const active = findLeaf(tab.root, tab.activePaneId)
  if (active?.kind === 'ssh') return active
  const leaves = allLeaves(tab.root).filter((l) => l.kind === 'ssh')
  return leaves.find((l) => l.status === 'connected') ?? leaves[0]
}



export default function App(): JSX.Element {
  const [servers, setServers] = useState<ServerConfig[]>([])
  const [tabs, setTabs] = useState<Tab[]>([])
  const [activeKey, setActiveKey] = useState<string | null>(null)
  const [editing, setEditing] = useState<ServerConfig | null | undefined>(undefined)
  const [showSettings, setShowSettings] = useState(false)
  const [showKeyGen, setShowKeyGen] = useState(false)
  const [showGroups, setShowGroups] = useState(false)
  const [broadcast, setBroadcast] = useState(false)
  const [sftpWidth, setSftpWidth] = useState(380)
  const [sidebarWidth, setSidebarWidth] = useState(270)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const prompts = useServerPrompts()
  // Вопросы про ключ хоста копим очередью: цепочка jump-хостов может спросить несколько раз.
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [multiExec, setMultiExec] = useState(false)
  const [showPuttyImport, setShowPuttyImport] = useState(true)
  const { settings, update } = useSettings()
  useWindowSnap()

  const tabsRef = useRef<Tab[]>([])
  tabsRef.current = tabs
  const activeKeyRef = useRef<string | null>(activeKey)
  activeKeyRef.current = activeKey
  const settingsRef = useRef(settings)
  settingsRef.current = settings
  const auxRestore = useAuxRestore()
  const persistSettingInit = useRef(false)
  const layoutReadyRef = useRef(false)
  const broadcastRef = useRef(broadcast)
  broadcastRef.current = broadcast
  const reconnectAttempts = useRef(new Map<string, number>())
  const reconnectTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>())

  useEffect(() => listenAuxGeoEvents(), [])

  useEffect(() => {
    void isWindowsPlatform().then(setShowPuttyImport)
  }, [])

  useEffect(() => {
    const t = window.setTimeout(() => window.dispatchEvent(new Event('resize')), 100)
    return () => window.clearTimeout(t)
  }, [activeKey, sftpWidth, tabs.length])

  useEffect(() => {
    setAuxPersistEnabled(!!settings.restoreAuxOnStart)
    if (!persistSettingInit.current) {
      persistSettingInit.current = true
      return
    }
    if (settings.restoreAuxOnStart) flushAuxPersist()
  }, [settings.restoreAuxOnStart])

  useEffect(() => {
    const timers = reconnectTimers.current
    return () => {
      for (const t of timers.values()) clearTimeout(t)
      timers.clear()
    }
  }, [])

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

  // Патч одной панели внутри вкладки. Единственный способ поменять состояние панели
  // снаружи — через него, поэтому его же отдаём хуку переподключения.
  const patchPane = useCallback(
    (tabKey: string, paneId: string, patch: Parameters<typeof updateLeaf>[2]) => {
      setTabs((prev) =>
        prev.map((t) => (t.key === tabKey ? { ...t, root: updateLeaf(t.root, paneId, patch) } : t))
      )
    },
    []
  )

  const reconnect = useReconnect({ patchPane })

  const handleFail = useCallback(
    (paneId: string, message: string) => {
      const tab = tabsRef.current.find((t) => allLeaves(t.root).some((l) => l.id === paneId))
      if (!tab) return
      const attempts = reconnectAttempts.current.get(paneId) ?? 0
      if (settingsRef.current.autoReconnect && attempts > 0) {
        reconnect.schedule(tab.key, paneId)
        return
      }
      setTabs((prev) =>
        prev.map((t) =>
          t.key === tab.key
            ? { ...t, root: updateLeaf(t.root, paneId, { status: 'error', statusMsg: message, sessionId: undefined }) }
            : t
        )
      )
    },
    [reconnect]
  )



  useEffect(() => {
    const off = window.api.session.onStatus((p) => {
      setTabs((prev) =>
        prev.map((t) => ({ ...t, root: updateLeafBySession(t.root, p.id, { status: p.status, statusMsg: p.message }) }))
      )
    })
    const offExit = window.api.session.onExit((p) => {
      void window.api.session.close(p.id)
      let toReconnect: { tabKey: string; paneId: string } | null = null
      for (const t of tabsRef.current) {
        const leaf = allLeaves(t.root).find((l) => l.sessionId === p.id)
        if (!leaf) continue
        if (shouldAutoReconnect(p, leaf, settingsRef.current)) {
          toReconnect = { tabKey: t.key, paneId: leaf.id }
        }
        break
      }
      if (toReconnect) {
        reconnect.schedule(toReconnect.tabKey, toReconnect.paneId)
        return
      }
      setTabs((prev) =>
        prev.map((t) => ({
          ...t,
          root: updateLeafBySession(t.root, p.id, {
            status: 'closed',
            statusMsg: p.error ?? 'Сессия завершена',
            sessionId: undefined
          })
        }))
      )
    })
    return () => {
      off()
      offExit()
    }
  }, [reconnect])

  const openServerTab = useCallback((server: ServerConfig) => {
    const leaf = makeLeaf(paneKindOf(server), server.name, server.id)
    const key = uid()
    setTabs((prev) => [...prev, { key, title: server.name, kind: 'terminal', root: leaf, activePaneId: leaf.id, sftpOpen: false, workspace: 'terminal' }])
    setActiveKey(key)
    setSidebarCollapsed(true)
  }, [])

  const openLocalTab = useCallback(() => {
    const leaf = makeLeaf('local', 'Локальный терминал')
    const key = uid()
    setTabs((prev) => [...prev, { key, title: 'Локальный терминал', kind: 'terminal', root: leaf, activePaneId: leaf.id, sftpOpen: false, workspace: 'terminal' }])
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
      { key, title: fileName, kind: 'editor', root: leaf, activePaneId: leaf.id, sftpOpen: false, workspace: 'terminal', editor: { sessionId, remotePath } }
    ])
    setActiveKey(key)
  }, [])

  useEffect(() => {
    let off: (() => void) | undefined
    void import('@tauri-apps/api/event').then(({ listen }) => {
      void listen<{ sessionId: string; remotePath: string }>('serein-open-editor', (e) => {
        openEditorTab(e.payload.sessionId, e.payload.remotePath)
      }).then((u) => {
        off = u
      })
    })
    return () => off?.()
  }, [openEditorTab])

  const reattachTabFromAux = useCallback((p: ReattachTabPayload) => {
    // Сессию забираем себе в любом случае: окно, которое её вернуло, сейчас закроется.
    void takeOverSession(p.sessionId)
    const next = planReattach(tabsRef.current, p)
    setTabs(next.tabs)
    setActiveKey(next.activeKey)
  }, [])

  const reattachWorkspaceFromAux = useCallback(
    (p: ReattachWorkspacePayload) => {
      const existing = findTabKeyBySession(tabsRef.current, p.sessionId)
      if (existing) {
        setTabs((prev) => prev.map((t) => (t.key === existing ? { ...t, workspace: p.tool } : t)))
        setActiveKey(existing)
        return
      }
      reattachTabFromAux({
        sessionId: p.sessionId,
        serverId: p.serverId,
        title: p.title,
        workspace: p.tool,
        sftpOpen: false,
        kind: 'ssh'
      })
    },
    [reattachTabFromAux]
  )

  const reattachSftpFromAux = useCallback(
    (p: ReattachSftpPayload) => {
      const existing = findTabKeyBySession(tabsRef.current, p.sessionId)
      if (existing) {
        setTabs((prev) =>
          prev.map((t) => (t.key === existing ? { ...t, sftpOpen: true, workspace: 'terminal' } : t))
        )
        setActiveKey(existing)
        return
      }
      void (async () => {
        const list = await window.api.servers.list()
        const srv = p.serverId ? list.find((s) => s.id === p.serverId) : undefined
        reattachTabFromAux({
          sessionId: p.sessionId,
          serverId: p.serverId,
          title: srv?.name ?? 'SSH',
          workspace: 'terminal',
          sftpOpen: true,
          kind: 'ssh'
        })
      })()
    },
    [reattachTabFromAux]
  )

  useEffect(() => {
    const offs: (() => void)[] = []
    void import('@tauri-apps/api/event').then(({ listen }) => {
      void listen<ReattachTabPayload>('serein-reattach-tab', (e) => {
        reattachTabFromAux(e.payload)
      }).then((u) => offs.push(u))
      void listen<ReattachWorkspacePayload>('serein-reattach-workspace', (e) => {
        reattachWorkspaceFromAux(e.payload)
      }).then((u) => offs.push(u))
      void listen<ReattachSftpPayload>('serein-reattach-sftp', (e) => {
        reattachSftpFromAux(e.payload)
      }).then((u) => offs.push(u))
    })
    return () => {
      for (const u of offs) u()
    }
  }, [reattachTabFromAux, reattachWorkspaceFromAux, reattachSftpFromAux])

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
        // Вид панели берём из профиля: при split сервер с COM-портом или telnet
        // иначе открывался бы как SSH и подключение падало на первом же шаге.
        let fresh
        if (choice.kind === 'local') {
          fresh = makeLeaf('local', 'Локальный терминал')
        } else {
          const srv = servers.find((x) => x.id === choice.serverId)
          fresh = makeLeaf(srv ? paneKindOf(srv) : 'ssh', choice.title, choice.serverId)
        }
        return { ...t, root: splitLeaf(t.root, cur.id, dir, fresh), activePaneId: fresh.id }
      })
    )
  }, [servers])

  const focusPane = useCallback((tabKey: string, paneId: string) => {
    setTabs((prev) => prev.map((t) => (t.key === tabKey ? { ...t, activePaneId: paneId } : t)))
  }, [])

  const closePane = useCallback((tabKey: string, paneId: string) => {
    const tab = tabsRef.current.find((t) => t.key === tabKey)
    const leaf = tab && findLeaf(tab.root, paneId)
    if (leaf?.sessionId) window.api.session.close(leaf.sessionId)
    reconnect.clear(paneId)
    setTabs((prev) => {
      const { tabs: next } = closePaneIn(prev, tabKey, paneId)
      setActiveKey((cur) => (next.some((t) => t.key === cur) ? cur : next.length ? next[next.length - 1].key : null))
      return next
    })
  }, [reconnect])

  const resizeSplit = useCallback((tabKey: string, splitId: string, sizes: [number, number]) => {
    setTabs((prev) => prev.map((t) => (t.key === tabKey ? { ...t, root: updateSplitSizes(t.root, splitId, sizes) } : t)))
  }, [])

  const handleReady = useCallback(
    (paneId: string, sessionId: string) => {
      const timer = reconnectTimers.current.get(paneId)
      if (timer) {
        clearTimeout(timer)
        reconnectTimers.current.delete(paneId)
      }
      reconnectAttempts.current.delete(paneId)
      let serverId: string | undefined
      for (const tab of tabsRef.current) {
        const leaf = findLeaf(tab.root, paneId)
        if (leaf?.serverId) {
          serverId = leaf.serverId
          break
        }
      }
      setTabs((prev) => prev.map((t) => ({ ...t, root: updateLeaf(t.root, paneId, { sessionId }) })))
      if (serverId) auxRestore.tryRestoreFor(serverId, sessionId)
    },
    [auxRestore]
  )

  // Broadcast ограничен панелями ТЕКУЩЕЙ вкладки (а не всеми вкладками) —
  // чтобы случайно не отправить команду в прод-сессию из другой вкладки.
  const broadcastInput = useCallback((fromId: string, data: string) => {
    if (!broadcastRef.current) return
    for (const id of broadcastTargets(tabsRef.current, fromId)) {
      window.api.session.write(id, data)
    }
  }, [])

  // Сколько сессий получит broadcast-ввод (для индикатора): панели активной вкладки.
  const broadcastTargetCount = useMemo(() => {
    const tab = tabs.find((t) => t.key === activeKey)
    if (!tab) return 0
    return allLeaves(tab.root).filter((l) => l.sessionId).length
  }, [tabs, activeKey])

  // Живой статус подключения по серверу (агрегируем по всем вкладкам/панелям).
  const serverStatuses = useMemo(() => {
    const rank: Record<string, number> = { connected: 4, reconnecting: 3, connecting: 2, error: 1 }
    const out: Record<string, 'connected' | 'connecting' | 'reconnecting' | 'error'> = {}
    for (const t of tabs) {
      for (const l of allLeaves(t.root)) {
        if (l.kind !== 'ssh' || !l.serverId) continue
        const st =
          l.status === 'connected'
            ? 'connected'
            : l.status === 'reconnecting'
              ? 'reconnecting'
              : l.status === 'error'
                ? 'error'
                : l.status === 'connecting'
                  ? 'connecting'
                  : null
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
    if (tab) {
      for (const l of allLeaves(tab.root)) {
        if (l.sessionId) window.api.session.close(l.sessionId)
        reconnect.clear(l.id)
      }
    }
    setTabs((prev) => {
      const next = prev.filter((t) => t.key !== key)
      setActiveKey((cur) => (cur !== key ? cur : next.length ? next[next.length - 1].key : null))
      return next
    })
  }, [reconnect])

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

  const setWorkspace = useCallback((key: string, tool: WorkspaceTool) => {
    setTabs((prev) => prev.map((t) => (t.key === key ? { ...t, workspace: tool } : t)))
  }, [])

  const toggleSftp = useCallback((key: string) => {
    setTabs((prev) =>
      prev.map((t) => {
        if (t.key !== key) return t
        if (t.sftpOpen) return { ...t, sftpOpen: false }
        return { ...t, sftpOpen: true, workspace: 'terminal' }
      })
    )
  }, [])

  const detachTab = useCallback(
    async (key: string) => {
      const tab = tabsRef.current.find((t) => t.key === key)
      if (!tab || tab.kind !== 'terminal') return
      const leaves = allLeaves(tab.root)
      if (leaves.length > 1) {
        alert('Открепление пока поддерживается только для вкладки с одной панелью.')
        return
      }
      const leaf = leaves[0]
      if (!leaf?.sessionId || leaf.status !== 'connected') return
      if (leaf.kind === 'ssh' && !leaf.serverId) return

      // Владение передаём ДО открытия окна: иначе наш собственный терминал, размонтируясь
      // при удалении вкладки, успел бы закрыть сессию, которая уже уехала.
      await handOverSession(leaf.sessionId, detachedWindowLabel(leaf.sessionId))
      try {
        await openDetachedTabWindow({
          sessionId: leaf.sessionId,
          serverId: leaf.serverId,
          title: tab.title,
          workspace: tab.workspace,
          sftpOpen: tab.sftpOpen,
          kind: leaf.kind
        })
      } catch {
        // Окно не открылось — забираем сессию обратно, иначе её некому будет закрыть.
        await takeOverSession(leaf.sessionId)
        return
      }

      reconnect.clear(leaf.id)
      setTabs((prev) => {
        const next = prev.filter((t) => t.key !== key)
        setActiveKey((cur) => (cur !== key ? cur : next.length ? next[next.length - 1].key : null))
        return next
      })
    },
    [reconnect]
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

  // Запуск: вкладки, доп. панели, локальный терминал.
  const startedOnceRef = useRef(false)
  useEffect(() => {
    if (startedOnceRef.current) return
    startedOnceRef.current = true
    void (async () => {
      const s = await window.api.settings.get()
      const saved = await window.api.layout.get()
      const serverList = await window.api.servers.list()
      setServers(serverList)
      setAuxPersistEnabled(!!s.restoreAuxOnStart)
      let auxWindows: SavedAuxWindow[] = []
      if (s.restoreAuxOnStart) {
        try {
          const aux = await window.api.auxLayout.get()
          auxWindows = Array.isArray(aux?.windows) ? aux.windows : []
        } catch {
          auxWindows = []
        }
        seedAuxLive(auxWindows)
        auxRestore.remember(auxWindows)
      }
      markAuxPersistReady()

      // Что именно открыть — решает чистая функция; здесь остаётся только применить.
      const restored = planRestore({
        restoreTabs: !!s.restoreTabsOnStart,
        restoreAux: !!s.restoreAuxOnStart,
        saved,
        auxWindows,
        servers: serverList
      })
      if (restored.length) {
        setTabs(restored)
        setActiveKey(restored[restored.length - 1].key)
        if (restored.some((t) => allLeaves(t.root).some((l) => l.kind === 'ssh'))) {
          setSidebarCollapsed(true)
        }
        layoutReadyRef.current = true
        return
      }
      if (s.openLocalOnStart) openLocalTab()
      layoutReadyRef.current = true
    })()
  }, [openLocalTab])

  // Сохранение раскладки терминальных вкладок (с дебаунсом) для восстановления.
  useEffect(() => {
    if (!layoutReadyRef.current) return
    const id = setTimeout(() => {
      const payload = tabs
        .filter((t) => t.kind === 'terminal')
        .map((t) => ({
          title: t.title,
          root: serializePane(t.root),
          sftpOpen: t.sftpOpen,
          workspace: t.workspace
        }))
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
        alert(
          r.imported
            ? `Импортировано серверов: ${r.imported}`
            : 'Новых серверов не нашлось — всё уже есть в списке.'
        )
      } catch (e) {
        // Частый случай — импортировать просто неоткуда; это не поломка, а факт.
        alert(errText(e))
      }
    },
    [reloadServers]
  )

  const groupOrder = useMemo(() => settings.groupOrder ?? [], [settings.groupOrder])
  const collapsedGroups = useMemo(() => settings.collapsedGroups ?? [], [settings.collapsedGroups])

  /** Группы из настроек плюс те, что встречаются у серверов, — вторые могли приехать импортом. */
  const allGroups = useMemo(() => {
    const fromServers = servers
      .map((s) => s.group?.trim())
      .filter((g): g is string => !!g)
    return [...new Set([...groupOrder, ...fromServers])]
  }, [groupOrder, servers])

  const saveGroupOrder = useCallback(
    (next: string[]) => {
      update({ groupOrder: next })
    },
    [update]
  )

  const toggleGroup = useCallback(
    (group: string) => {
      const cur = settingsRef.current.collapsedGroups ?? []
      update({
        collapsedGroups: cur.includes(group) ? cur.filter((g) => g !== group) : [...cur, group]
      })
    },
    [update]
  )

  const createGroup = useCallback(() => {
    const name = prompt('Название новой группы')?.trim()
    if (!name) return
    const cur = settingsRef.current.groupOrder ?? []
    if (cur.includes(name)) {
      alert(`Группа «${name}» уже есть`)
      return
    }
    update({ groupOrder: [...cur, name] })
  }, [update])

  /**
   * Перенос сервера: пересчитываем позиции всей целевой группы, а не только
   * перетащенного, — иначе после нескольких перестановок порядок «слипается».
   */
  const dropServer = useCallback(
    async (serverId: string, group: string, index?: number) => {
      const moved = servers.find((s) => s.id === serverId)
      if (!moved) return
      const target = servers
        .filter((s) => s.id !== serverId && (s.group?.trim() || '') === group)
        .sort((a, b) => (a.order ?? 0) - (b.order ?? 0) || a.name.localeCompare(b.name))
      const at = index === undefined ? target.length : Math.max(0, Math.min(index, target.length))
      target.splice(at, 0, moved)

      // Новый порядок показываем сразу, не дожидаясь записи в хранилище: запись
      // идёт через IPC и приезжает кадров через несколько, а список всё это время
      // стоял бы в старом порядке — на глаз это рывок сразу после броска.
      const order = new Map(target.map((s, i) => [s.id, i]))
      setServers((prev) =>
        prev.map((s) => (order.has(s.id) ? { ...s, group, order: order.get(s.id)! } : s))
      )

      await window.api.servers.reorder(target.map((s, i) => ({ id: s.id, group, order: i })))
      await reloadServers()
    },
    [servers, reloadServers]
  )

  // Сайдбар присылает готовый порядок целиком: только он знает, что нарисовал
  // (в списке есть и группы, которых ещё нет в настройках).
  const dropGroup = useCallback(
    (order: string[]) => {
      update({ groupOrder: order })
    },
    [update]
  )

  const renameGroup = useCallback(
    async (from: string, to: string) => {
      const members = servers.filter((s) => (s.group?.trim() || '') === from)
      if (members.length) {
        await window.api.servers.reorder(
          members.map((s, i) => ({ id: s.id, group: to, order: s.order ?? i }))
        )
      }
      const cur = settingsRef.current.groupOrder ?? []
      update({ groupOrder: cur.map((g) => (g === from ? to : g)) })
      await reloadServers()
    },
    [servers, update, reloadServers]
  )

  const deleteGroup = useCallback(
    async (group: string) => {
      const members = servers.filter((s) => (s.group?.trim() || '') === group)
      if (members.length) {
        // Серверы не удаляем — только вынимаем из группы.
        await window.api.servers.reorder(members.map((s, i) => ({ id: s.id, group: '', order: i })))
      }
      const cur = settingsRef.current.groupOrder ?? []
      update({ groupOrder: cur.filter((g) => g !== group) })
      await reloadServers()
    },
    [servers, update, reloadServers]
  )

  const moveGroup = useCallback(
    (group: string, dir: -1 | 1) => {
      const cur = [...(settingsRef.current.groupOrder ?? allGroups)]
      const i = cur.indexOf(group)
      const j = i + dir
      if (i < 0 || j < 0 || j >= cur.length) return
      ;[cur[i], cur[j]] = [cur[j], cur[i]]
      update({ groupOrder: cur })
    },
    [update, allGroups]
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
    const activeTab = tabs.find((t) => t.key === activeKey)
    if (activeTab?.kind === 'terminal') {
      const sshLeaf = sshLeafForTools(activeTab)
      if (sshLeaf) {
        const tools: { id: WorkspaceTool; label: string }[] = [
          { id: 'terminal', label: 'Terminal' },
          { id: 'docker', label: 'Docker' },
          { id: 'logs', label: 'Logs' },
          { id: 'processes', label: 'Processes' },
          { id: 'services', label: 'Services' },
          { id: 'tunnels', label: 'Tunnels' }
        ]
        for (const x of tools) {
          items.push({
            id: 'ws:' + x.id,
            label: x.label,
            hint: 'инструмент сервера',
            icon: '▣',
            group: 'Рабочее место',
            run: () => setWorkspace(activeTab.key, x.id)
          })
        }
      }
    }
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
  }, [servers, tabs, activeKey, openServerTab, openLocalTab, setWorkspace])

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
        showPuttyImport={showPuttyImport}
        width={sidebarWidth}
        collapsed={sidebarCollapsed}
        onToggleCollapse={() => setSidebarCollapsed((v) => !v)}
        statuses={serverStatuses}
        groupOrder={allGroups}
        collapsedGroups={collapsedGroups}
        onToggleGroup={toggleGroup}
        onNewGroup={createGroup}
        onOpenGroups={() => setShowGroups(true)}
        onMultiExec={() => setMultiExec(true)}
        onDropServer={(id, group, index) => void dropServer(id, group, index)}
        onDropGroup={dropGroup}
      />

      {!sidebarCollapsed && <div className="sidebar-resizer" onMouseDown={startSidebarResize} />}

      <div className="workspace">
        <TabBar
          tabs={tabs}
          activeKey={activeKey}
          servers={servers}
          onSelect={setActiveKey}
          onClose={closeTab}
          onNewLocal={openLocalTab}
          onToggleSftp={toggleSftp}
          onSetWorkspace={setWorkspace}
          onDetachTab={(key) => void detachTab(key)}
          onRename={renameTab}
          onReorder={reorderTabs}
          onSplit={splitPane}
          broadcast={broadcast}
          onToggleBroadcast={() => setBroadcast((b) => !b)}
        />

        <div className="terminals">
          {tabs.length === 0 && servers.length === 0 && (
            <div className="empty-state">
              <h1>Serein</h1>
              <p>
                Серверов пока нет. Быстрее всего — забрать те, что уже
                <br />
                настроены на этой машине.
              </p>
              <div className="welcome-actions">
                <button onClick={() => void importServers('ssh')}>Импорт из ~/.ssh/config</button>
                {showPuttyImport && (
                  <button onClick={() => void importServers('putty')}>Импорт сессий PuTTY</button>
                )}
                <button onClick={() => setEditing(null)}>Добавить сервер вручную</button>
              </div>
              <p className="welcome-hint">Локальный терминал работает и без настройки.</p>
              <button className="ghost" onClick={openLocalTab}>
                Открыть локальный терминал
              </button>
            </div>
          )}

          {tabs.length === 0 && servers.length > 0 && (
            <div className="empty-state">
              <h1>Serein</h1>
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
            const sshLeaf = sshLeafForTools(tab)
            const showRail = !!sshLeaf
            const tool = showRail ? tab.workspace : 'terminal'
            const sessionId =
              sshLeaf && sshLeaf.status === 'connected' ? sshLeaf.sessionId : undefined
            const goTerminal = (): void => setWorkspace(tab.key, 'terminal')
            const panelTitle = tab.title
            return (
              <div
                key={tab.key}
                className="terminal-slot"
                style={{ display: isActive ? 'flex' : 'none' }}
              >
                <ServerWorkspace
                  leaf={showRail && sshLeaf ? sshLeaf : undefined}
                  server={servers.find((srv) => srv.id === sshLeaf?.serverId)}
                  title={tab.title}
                  panelTitle={panelTitle}
                  tool={tool}
                  onSelectTool={(t) => setWorkspace(tab.key, t)}
                  onReconnect={() => sshLeaf && reconnect.now(tab.key, sshLeaf.id)}
                  onEditServer={() => {
                    const srv = servers.find((x) => x.id === sshLeaf?.serverId)
                    if (srv) setEditing(srv)
                  }}
                  sessionId={sessionId}
                  serverId={sshLeaf?.serverId}
                  sftpOpen={tab.sftpOpen}
                  sftpWidth={sftpWidth}
                  onSftpClose={() => toggleSftp(tab.key)}
                  onSftpResizeStart={startSftpResize}
                  onOpenInEditor={sessionId ? (rp) => openEditorTab(sessionId, rp) : undefined}
                  onDetached={goTerminal}
                  terminal={
                    <PaneView
                      node={tab.root}
                      activePaneId={tab.activePaneId}
                      tabActive={tab.key === activeKey}
                      canClose={allLeaves(tab.root).length > 1}
                      onFocusPane={(pid) => focusPane(tab.key, pid)}
                      onReady={handleReady}
                      onFail={handleFail}
                      onInput={broadcastInput}
                      onClosePane={(pid) => closePane(tab.key, pid)}
                      onReconnect={(pid) => reconnect.now(tab.key, pid)}
                      onCancelReconnect={(pid) => reconnect.cancel(tab.key, pid)}
                      onResizeSplit={(sid, sizes) => resizeSplit(tab.key, sid, sizes)}
                    />
                  }
                />
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
          return <StatusBar leaf={activeLeaf} server={srv} broadcast={broadcast} broadcastTargets={broadcastTargetCount} />
        })()}
      </div>

      {editing !== undefined && (
        <ServerForm initial={editing} servers={servers} onCancel={() => setEditing(undefined)} onSave={saveServer} />
      )}

      {paletteOpen && <CommandPalette items={paletteItems} onClose={() => setPaletteOpen(false)} />}

      {showSettings && <SettingsModal onClose={() => setShowSettings(false)} />}

      {multiExec && <MultiExecModal servers={servers} onClose={() => setMultiExec(false)} />}

      {showGroups && (
        <GroupsModal
          groups={allGroups}
          servers={servers}
          onClose={() => setShowGroups(false)}
          onRename={(from, to) => void renameGroup(from, to)}
          onDelete={(g) => void deleteGroup(g)}
          onCreate={(name) => saveGroupOrder([...(settingsRef.current.groupOrder ?? []), name])}
          onAssign={(id, group) => void dropServer(id, group)}
          onMove={moveGroup}
        />
      )}

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

      {prompts.hostKeyQueue.length > 0 && (
        <HostKeyModal request={prompts.hostKeyQueue[0]} onAnswer={prompts.answerHostKey} />
      )}

      {prompts.ki && (
        <KiModal
          sessionId={prompts.ki.id}
          prompts={prompts.ki.prompts}
          onSubmit={prompts.answerKi}
          onCancel={prompts.cancelKi}
        />
      )}
    </div>
  )
}

export type { PaneLeaf }
