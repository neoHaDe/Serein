import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { MouseEvent as ReactMouseEvent } from 'react'
import type { KIPrompt, SavedAuxWindow, SerializedPane, SerializedTab, ServerConfig, WorkspaceTool } from '../shared/types'
import { parseWorkspaceTool } from '../shared/types'
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
import { WorkspaceRail } from './components/WorkspaceRail'
import { DockerPanel } from './components/DockerPanel'
import { ProcessPanel } from './components/ProcessPanel'
import { ServicePanel } from './components/ServicePanel'
import { HostLogsPanel } from './components/HostLogsPanel'
import { openDetachedTabWindow } from './components/DetachedTabWindow'
import { TunnelPanel } from './components/TunnelPanel'
import { markSessionDetached, clearDetachedMark } from './detachedSessions'
import type { ReattachSftpPayload, ReattachTabPayload, ReattachWorkspacePayload } from './reattach'
import { useSettings } from './SettingsContext'
import { bindingLookup, comboFromEvent } from './keybindings'
import { applyUiTheme } from './themes'
import { useWindowSnap } from './windowSnap'
import { openAuxWindow, sanitizeWindowLabel } from './auxWindows'
import {
  auxWindowKey,
  flushAuxPersist,
  listenAuxGeoEvents,
  markAuxPersistReady,
  seedAuxLive,
  setAuxPersistEnabled
} from './auxLayout'
import { openDetachedLogsWindow } from './components/dockerLogs'
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
  workspace: WorkspaceTool
  /** Для kind==='editor': какой файл и на какой сессии редактируется. */
  editor?: { sessionId: string; remotePath: string }
  editorDirty?: boolean
}

/** Что открыть в новой панели при сплите: локальный терминал или конкретный сервер. */
export type SplitChoice = { kind: 'local' } | { kind: 'ssh'; serverId: string; title: string }

function uid(): string {
  return crypto.randomUUID()
}

function serializedHasServer(node: SerializedPane, serverId: string): boolean {
  if (node.t === 'leaf') return node.serverId === serverId
  return serializedHasServer(node.children[0], serverId) || serializedHasServer(node.children[1], serverId)
}

function tabFromSaved(st: SerializedTab, restoreWorkspace: boolean): Tab {
  const root = deserializePane(st.root)
  const workspace = restoreWorkspace ? parseWorkspaceTool(st.workspace, st.sftpOpen) : 'terminal'
  const legacyWs = st.workspace as string | undefined
  const sftpOpen = restoreWorkspace && (!!st.sftpOpen || legacyWs === 'files')
  return {
    key: uid(),
    title: st.title,
    kind: 'terminal',
    root,
    activePaneId: firstLeaf(root).id,
    sftpOpen,
    workspace
  }
}

/** SSH-лист для рельсы: активная панель, иначе первый подключённый SSH. */
function sshLeafForTools(tab: Tab): PaneLeaf | undefined {
  const active = findLeaf(tab.root, tab.activePaneId)
  if (active?.kind === 'ssh') return active
  const leaves = allLeaves(tab.root).filter((l) => l.kind === 'ssh')
  return leaves.find((l) => l.status === 'connected') ?? leaves[0]
}

function findTabKeyBySession(tabs: Tab[], sessionId: string): string | undefined {
  for (const t of tabs) {
    if (t.kind !== 'terminal') continue
    for (const l of allLeaves(t.root)) {
      if (l.sessionId === sessionId) return t.key
    }
  }
  return undefined
}

const RECONNECT_MAX = 5

export default function App(): JSX.Element {
  const [servers, setServers] = useState<ServerConfig[]>([])
  const [tabs, setTabs] = useState<Tab[]>([])
  const [activeKey, setActiveKey] = useState<string | null>(null)
  const [editing, setEditing] = useState<ServerConfig | null | undefined>(undefined)
  const [showSettings, setShowSettings] = useState(false)
  const [showKeyGen, setShowKeyGen] = useState(false)
  const [broadcast, setBroadcast] = useState(false)
  const [sftpWidth, setSftpWidth] = useState(380)
  const [sidebarWidth, setSidebarWidth] = useState(270)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [kiRequest, setKiRequest] = useState<{ id: string; prompts: KIPrompt[] } | null>(null)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const { settings, update } = useSettings()
  useWindowSnap()

  const tabsRef = useRef<Tab[]>([])
  tabsRef.current = tabs
  const activeKeyRef = useRef<string | null>(activeKey)
  activeKeyRef.current = activeKey
  const settingsRef = useRef(settings)
  settingsRef.current = settings
  const pendingAuxRef = useRef<SavedAuxWindow[]>([])
  const openedAuxRef = useRef(new Set<string>())
  const persistSettingInit = useRef(false)
  const layoutReadyRef = useRef(false)
  const broadcastRef = useRef(broadcast)
  broadcastRef.current = broadcast
  const reconnectAttempts = useRef(new Map<string, number>())
  const reconnectTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>())

  useEffect(() => listenAuxGeoEvents(), [])

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

  // Обновление статуса/завершения сессий — патчим соответствующий лист по sessionId.
  const clearReconnect = useCallback((paneId: string) => {
    const timer = reconnectTimers.current.get(paneId)
    if (timer) {
      clearTimeout(timer)
      reconnectTimers.current.delete(paneId)
    }
    reconnectAttempts.current.delete(paneId)
  }, [])

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

  const scheduleReconnect = useCallback(
    (tabKey: string, paneId: string) => {
      const prevTimer = reconnectTimers.current.get(paneId)
      if (prevTimer) {
        clearTimeout(prevTimer)
        reconnectTimers.current.delete(paneId)
      }
      const n = (reconnectAttempts.current.get(paneId) ?? 0) + 1
      if (n > RECONNECT_MAX) {
        reconnectAttempts.current.delete(paneId)
        setTabs((prev) =>
          prev.map((t) =>
            t.key === tabKey
              ? {
                  ...t,
                  root: updateLeaf(t.root, paneId, {
                    status: 'error',
                    statusMsg: `Не удалось переподключить после ${RECONNECT_MAX} попыток`,
                    sessionId: undefined
                  })
                }
              : t
          )
        )
        return
      }
      reconnectAttempts.current.set(paneId, n)
      const delay = Math.min(15_000, 1000 * 2 ** (n - 1))
      setTabs((prev) =>
        prev.map((t) =>
          t.key === tabKey
            ? {
                ...t,
                root: updateLeaf(t.root, paneId, {
                  status: 'reconnecting',
                  statusMsg: `Переподключение… попытка ${n}/${RECONNECT_MAX}`,
                  sessionId: undefined
                })
              }
            : t
        )
      )
      const timer = setTimeout(() => {
        reconnectTimers.current.delete(paneId)
        reconnectPane(tabKey, paneId)
      }, delay)
      reconnectTimers.current.set(paneId, timer)
    },
    [reconnectPane]
  )

  const reconnectPaneManual = useCallback(
    (tabKey: string, paneId: string) => {
      const timer = reconnectTimers.current.get(paneId)
      if (timer) {
        clearTimeout(timer)
        reconnectTimers.current.delete(paneId)
      }
      reconnectAttempts.current.delete(paneId)
      reconnectPane(tabKey, paneId)
    },
    [reconnectPane]
  )

  const cancelReconnect = useCallback(
    (tabKey: string, paneId: string) => {
      clearReconnect(paneId)
      setTabs((prev) =>
        prev.map((t) =>
          t.key === tabKey
            ? {
                ...t,
                root: updateLeaf(t.root, paneId, {
                  status: 'closed',
                  statusMsg: 'Переподключение отменено',
                  sessionId: undefined
                })
              }
            : t
        )
      )
    },
    [clearReconnect]
  )

  const handleFail = useCallback(
    (paneId: string, message: string) => {
      const tab = tabsRef.current.find((t) => allLeaves(t.root).some((l) => l.id === paneId))
      if (!tab) return
      const attempts = reconnectAttempts.current.get(paneId) ?? 0
      if (settingsRef.current.autoReconnect && attempts > 0) {
        scheduleReconnect(tab.key, paneId)
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
    [scheduleReconnect]
  )

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
      void window.api.session.close(p.id)
      let reconnect: { tabKey: string; paneId: string } | null = null
      for (const t of tabsRef.current) {
        const leaf = allLeaves(t.root).find((l) => l.sessionId === p.id)
        if (!leaf) continue
        const canAuto =
          p.reason === 'drop' &&
          leaf.kind === 'ssh' &&
          settingsRef.current.autoReconnect &&
          (leaf.status === 'connected' || leaf.status === 'connecting' || leaf.status === 'reconnecting')
        if (canAuto) reconnect = { tabKey: t.key, paneId: leaf.id }
        break
      }
      if (reconnect) {
        scheduleReconnect(reconnect.tabKey, reconnect.paneId)
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
  }, [scheduleReconnect])

  const openServerTab = useCallback((server: ServerConfig) => {
    const leaf = makeLeaf('ssh', server.name, server.id)
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
    const existing = findTabKeyBySession(tabsRef.current, p.sessionId)
    if (existing) {
      setTabs((prev) =>
        prev.map((t) =>
          t.key === existing ? { ...t, workspace: p.workspace, sftpOpen: p.sftpOpen, title: p.title } : t
        )
      )
      setActiveKey(existing)
      clearDetachedMark(p.sessionId)
      return
    }
    markSessionDetached(p.sessionId)
    const leaf = makeLeaf(p.kind, p.title, p.serverId)
    const root: PaneLeaf = { ...leaf, sessionId: p.sessionId, status: 'connected' }
    const key = uid()
    setTabs((prev) => [
      ...prev,
      {
        key,
        title: p.title,
        kind: 'terminal',
        root,
        activePaneId: root.id,
        sftpOpen: p.sftpOpen,
        workspace: p.workspace
      }
    ])
    setActiveKey(key)
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
    clearReconnect(paneId)
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
  }, [clearReconnect])

  const resizeSplit = useCallback((tabKey: string, splitId: string, sizes: [number, number]) => {
    setTabs((prev) => prev.map((t) => (t.key === tabKey ? { ...t, root: updateSplitSizes(t.root, splitId, sizes) } : t)))
  }, [])

  const openSavedAux = useCallback(async (w: SavedAuxWindow, sessionId: string) => {
    try {
      if (w.kind === 'sftp') {
        await openAuxWindow({
          label: 'sftp-' + sanitizeWindowLabel(sessionId),
          query: { sftp: '1', sessionId },
          title: 'SFTP',
          width: Math.max(w.w || 480, 420),
          height: Math.max(w.h || 720, 280),
          x: w.x,
          y: w.y,
          persist: { kind: 'sftp', serverId: w.serverId }
        })
      } else if (w.containerId) {
        await openDetachedLogsWindow({
          sessionId,
          serverId: w.serverId,
          containerId: w.containerId,
          name: w.name ?? w.containerId,
          width: Math.max(w.w || 800, 420),
          height: Math.max(w.h || 560, 280),
          x: w.x,
          y: w.y
        })
      }
    } catch {
      openedAuxRef.current.delete(auxWindowKey(w))
    }
  }, [])

  const tryRestoreAuxFor = useCallback(
    (serverId: string, sessionId: string) => {
      for (const w of pendingAuxRef.current) {
        if (w.serverId !== serverId) continue
        const k = auxWindowKey(w)
        if (openedAuxRef.current.has(k)) continue
        openedAuxRef.current.add(k)
        void openSavedAux(w, sessionId)
      }
    },
    [openSavedAux]
  )

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
      clearDetachedMark(sessionId)
      if (serverId) tryRestoreAuxFor(serverId, sessionId)
    },
    [tryRestoreAuxFor]
  )

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
        clearReconnect(l.id)
      }
    }
    setTabs((prev) => {
      const next = prev.filter((t) => t.key !== key)
      setActiveKey((cur) => (cur !== key ? cur : next.length ? next[next.length - 1].key : null))
      return next
    })
  }, [clearReconnect])

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

      markSessionDetached(leaf.sessionId)
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
        clearDetachedMark(leaf.sessionId)
        return
      }

      clearReconnect(leaf.id)
      setTabs((prev) => {
        const next = prev.filter((t) => t.key !== key)
        setActiveKey((cur) => (cur !== key ? cur : next.length ? next[next.length - 1].key : null))
        return next
      })
    },
    [clearReconnect]
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
        pendingAuxRef.current = auxWindows
      }
      markAuxPersistReady()

      const restoreSftp = !!s.restoreAuxOnStart
      let restored: Tab[] = []
      if (s.restoreTabsOnStart && saved.length) {
        restored = saved.map((st) => tabFromSaved(st, true))
      } else if (restoreSftp) {
        const used = new Set<string>()
        for (const st of saved) {
          const ws = st.workspace as string | undefined
          if (!st.sftpOpen && ws !== 'files' && ws !== 'resources') continue
          restored.push(tabFromSaved(st, true))
          for (const l of allLeaves(deserializePane(st.root))) {
            if (l.serverId) used.add(l.serverId)
          }
        }
        for (const w of auxWindows) {
          if (!w.serverId || used.has(w.serverId)) continue
          const st = saved.find((x) => serializedHasServer(x.root, w.serverId))
          if (st) restored.push(tabFromSaved(st, false))
          else {
            const srv = serverList.find((x) => x.id === w.serverId)
            if (srv) {
              const leaf = makeLeaf('ssh', srv.name, srv.id)
              restored.push({
                key: uid(),
                title: srv.name,
                kind: 'terminal',
                root: leaf,
                activePaneId: leaf.id,
                sftpOpen: false,
                workspace: 'terminal'
              })
            }
          }
          used.add(w.serverId)
        }
      }
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
        width={sidebarWidth}
        collapsed={sidebarCollapsed}
        onToggleCollapse={() => setSidebarCollapsed((v) => !v)}
        statuses={serverStatuses}
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
                <button onClick={() => void importServers('putty')}>Импорт сессий PuTTY</button>
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
                {showRail && sshLeaf && (
                  <WorkspaceRail
                    title={tab.title}
                    leaf={sshLeaf}
                    server={servers.find((s) => s.id === sshLeaf.serverId)}
                    tool={tool}
                    onSelect={(t) => setWorkspace(tab.key, t)}
                    onReconnect={() => reconnectPaneManual(tab.key, sshLeaf.id)}
                    onEditServer={() => {
                      const srv = servers.find((s) => s.id === sshLeaf.serverId)
                      if (srv) setEditing(srv)
                    }}
                  />
                )}
                <div className="pane-area" style={{ display: tool === 'terminal' ? 'flex' : 'none' }}>
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
                    onReconnect={(pid) => reconnectPaneManual(tab.key, pid)}
                    onCancelReconnect={(pid) => cancelReconnect(tab.key, pid)}
                    onResizeSplit={(sid, sizes) => resizeSplit(tab.key, sid, sizes)}
                  />
                </div>
                {tool === 'terminal' && tab.sftpOpen && sessionId && (
                  <>
                    <div className="sftp-resizer" onMouseDown={startSftpResize} />
                    <SftpPanel
                      sessionId={sessionId}
                      serverId={sshLeaf?.serverId}
                      width={sftpWidth}
                      closing={false}
                      onClose={() => toggleSftp(tab.key)}
                      onOpenInEditor={(rp) => openEditorTab(sessionId, rp)}
                    />
                  </>
                )}
                {showRail && tool !== 'terminal' && (
                  <div className="ws-body">
                    {!sessionId && (
                      <div className="ws-waiting">Нет активного SSH-соединения</div>
                    )}
                    {sessionId && tool === 'docker' && (
                      <DockerPanel
                        sessionId={sessionId}
                        serverId={sshLeaf?.serverId}
                        panelTitle={panelTitle}
                        docked
                        onClose={goTerminal}
                        onGoTerminal={goTerminal}
                        onDetached={goTerminal}
                      />
                    )}
                    {sessionId && tool === 'logs' && (
                      <HostLogsPanel sessionId={sessionId} panelTitle={panelTitle} onDetached={goTerminal} />
                    )}
                    {sessionId && tool === 'processes' && (
                      <ProcessPanel sessionId={sessionId} panelTitle={panelTitle} onDetached={goTerminal} />
                    )}
                    {sessionId && tool === 'services' && (
                      <ServicePanel sessionId={sessionId} panelTitle={panelTitle} onDetached={goTerminal} />
                    )}
                    {sessionId && tool === 'tunnels' && (
                      <TunnelPanel
                        sessionId={sessionId}
                        server={servers.find((s) => s.id === sshLeaf?.serverId)}
                        panelTitle={panelTitle}
                        docked
                        onClose={goTerminal}
                        onDetached={goTerminal}
                        onEditServer={() => {
                          const srv = servers.find((s) => s.id === sshLeaf?.serverId)
                          if (srv) setEditing(srv)
                        }}
                      />
                    )}
                  </div>
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
