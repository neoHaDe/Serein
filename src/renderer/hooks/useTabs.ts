import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { AppSettings, SavedAuxWindow, ServerConfig, WorkspaceTool } from '../../shared/types'
import {
  markAuxPersistReady,
  seedAuxLive,
  setAuxPersistEnabled
} from '../auxLayout'
import { detachedWindowLabel, openDetachedTabWindow } from '../components/DetachedTabWindow'
import { handOverSession, takeOverSession } from '../detachedSessions'
import { bindingLookup, comboFromEvent } from '../keybindings'
import { allLeaves, findLeaf, updateLeaf, updateLeafBySession, updateSplitSizes } from '../paneTree'
import type { ReattachSftpPayload, ReattachTabPayload, ReattachWorkspacePayload } from '../reattach'
import { shouldAutoReconnect } from '../sessionRules'
import type { SplitChoice, Tab } from '../tabs'
import {
  broadcastTargets,
  canDetachTab,
  closePaneIn,
  findEditorTab,
  makeEditorTab,
  makeLocalTab,
  makeServerTab,
  nextActivePaneId,
  nextTabKey,
  openSftpOnTab,
  planReattach,
  planReattachSftp,
  planReattachWorkspace,
  planRestore,
  planSplitPane,
  reorderTabs,
  setTabWorkspace,
  tabsForLayoutPersist,
  toggleSftpOnTab
} from '../tabs'
import type { AuxRestore } from './useAuxRestore'
import { useReconnect, type Reconnect } from './useReconnect'

/**
 * Вкладки, панели, сессии и broadcast.
 *
 * Самый тяжёлый кусок бывшего `App.tsx`: состояние вкладок, подписки на события сессий,
 * reattach из откреплённых окон, горячие клавиши терминала, сохранение раскладки.
 */

export interface UseTabsOptions {
  servers: ServerConfig[]
  setServers: React.Dispatch<React.SetStateAction<ServerConfig[]>>
  settings: AppSettings
  auxRestore: AuxRestore
  onServerTabOpen: () => void
  onRestoredSshTabs: () => void
  onCommandPalette: () => void
  onOpenSettings: () => void
}

export interface TabsApi {
  tabs: Tab[]
  activeKey: string | null
  setActiveKey: React.Dispatch<React.SetStateAction<string | null>>
  broadcast: boolean
  setBroadcast: React.Dispatch<React.SetStateAction<boolean>>
  broadcastTargetCount: number
  reconnect: Reconnect
  openServerTab: (server: ServerConfig) => void
  openLocalTab: () => void
  openEditorTab: (sessionId: string, remotePath: string) => void
  closeTab: (key: string) => void
  renameTab: (key: string, title: string) => void
  reorderTabsByKey: (fromKey: string, toKey: string) => void
  splitPane: (tabKey: string, dir: 'row' | 'col', choice: SplitChoice) => void
  focusPane: (tabKey: string, paneId: string) => void
  closePane: (tabKey: string, paneId: string) => void
  resizeSplit: (tabKey: string, splitId: string, sizes: [number, number]) => void
  handleReady: (paneId: string, sessionId: string) => void
  handleFail: (paneId: string, message: string) => void
  broadcastInput: (fromId: string, data: string) => void
  setWorkspace: (key: string, tool: WorkspaceTool) => void
  toggleSftp: (key: string) => void
  detachTab: (key: string) => Promise<void>
  setEditorDirty: (key: string, dirty: boolean) => void
  connectedSessions: { sessionId: string; title: string }[]
}

export function useTabs({
  servers,
  setServers,
  settings,
  auxRestore,
  onServerTabOpen,
  onRestoredSshTabs,
  onCommandPalette,
  onOpenSettings
}: UseTabsOptions): TabsApi {
  const [tabs, setTabs] = useState<Tab[]>([])
  const [activeKey, setActiveKey] = useState<string | null>(null)
  const [broadcast, setBroadcast] = useState(false)

  const tabsRef = useRef<Tab[]>([])
  tabsRef.current = tabs
  const activeKeyRef = useRef<string | null>(activeKey)
  activeKeyRef.current = activeKey
  const settingsRef = useRef(settings)
  settingsRef.current = settings
  const broadcastRef = useRef(broadcast)
  broadcastRef.current = broadcast
  const layoutReadyRef = useRef(false)
  const startedOnceRef = useRef(false)

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
      if (settingsRef.current.autoReconnect) {
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

  const openServerTab = useCallback(
    (server: ServerConfig) => {
      const tab = makeServerTab(server)
      setTabs((prev) => [...prev, tab])
      setActiveKey(tab.key)
      onServerTabOpen()
    },
    [onServerTabOpen]
  )

  const openLocalTab = useCallback(() => {
    const tab = makeLocalTab()
    setTabs((prev) => [...prev, tab])
    setActiveKey(tab.key)
  }, [])

  const openEditorTab = useCallback((sessionId: string, remotePath: string) => {
    const existing = findEditorTab(tabsRef.current, sessionId, remotePath)
    if (existing) {
      setActiveKey(existing.key)
      return
    }
    const tab = makeEditorTab(sessionId, remotePath)
    setTabs((prev) => [...prev, tab])
    setActiveKey(tab.key)
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
    void takeOverSession(p.sessionId)
    const next = planReattach(tabsRef.current, p)
    setTabs(next.tabs)
    setActiveKey(next.activeKey)
  }, [])

  const reattachWorkspaceFromAux = useCallback(
    (p: ReattachWorkspacePayload) => {
      const plan = planReattachWorkspace(tabsRef.current, p)
      if (plan.action === 'focus') {
        setTabs((prev) => setTabWorkspace(prev, plan.key, plan.workspace))
        setActiveKey(plan.key)
        return
      }
      reattachTabFromAux(plan.payload)
    },
    [reattachTabFromAux]
  )

  const reattachSftpFromAux = useCallback(
    (p: ReattachSftpPayload) => {
      const existing = planReattachSftp(tabsRef.current, p)
      if (existing.action === 'focus') {
        setTabs((prev) => openSftpOnTab(prev, existing.key))
        setActiveKey(existing.key)
        return
      }
      void (async () => {
        const list = await window.api.servers.list()
        const srv = p.serverId ? list.find((s) => s.id === p.serverId) : undefined
        const plan = planReattachSftp(tabsRef.current, p, srv?.name)
        if (plan.action === 'focus') {
          setTabs((prev) => openSftpOnTab(prev, plan.key))
          setActiveKey(plan.key)
          return
        }
        reattachTabFromAux(plan.payload)
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

  const splitPane = useCallback(
    (tabKey: string, dir: 'row' | 'col', choice: SplitChoice) => {
      setTabs((prev) => prev.map((t) => (t.key === tabKey ? planSplitPane(t, servers, dir, choice) : t)))
    },
    [servers]
  )

  const focusPane = useCallback((tabKey: string, paneId: string) => {
    setTabs((prev) => prev.map((t) => (t.key === tabKey ? { ...t, activePaneId: paneId } : t)))
  }, [])

  const closePane = useCallback(
    (tabKey: string, paneId: string) => {
      const tab = tabsRef.current.find((t) => t.key === tabKey)
      const leaf = tab && findLeaf(tab.root, paneId)
      if (leaf?.sessionId) window.api.session.close(leaf.sessionId)
      reconnect.clear(paneId)
      setTabs((prev) => {
        const { tabs: next } = closePaneIn(prev, tabKey, paneId)
        setActiveKey((cur) => (next.some((t) => t.key === cur) ? cur : next.length ? next[next.length - 1].key : null))
        return next
      })
    },
    [reconnect]
  )

  const resizeSplit = useCallback((tabKey: string, splitId: string, sizes: [number, number]) => {
    setTabs((prev) => prev.map((t) => (t.key === tabKey ? { ...t, root: updateSplitSizes(t.root, splitId, sizes) } : t)))
  }, [])

  const handleReady = useCallback(
    (paneId: string, sessionId: string) => {
      reconnect.clear(paneId)
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
    [auxRestore, reconnect]
  )

  const broadcastInput = useCallback((fromId: string, data: string) => {
    if (!broadcastRef.current) return
    for (const id of broadcastTargets(tabsRef.current, fromId)) {
      window.api.session.write(id, data)
    }
  }, [])

  const broadcastTargetCount = useMemo(() => {
    const tab = tabs.find((t) => t.key === activeKey)
    if (!tab) return 0
    return allLeaves(tab.root).filter((l) => l.sessionId).length
  }, [tabs, activeKey])

  const closeTab = useCallback(
    (key: string) => {
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
    },
    [reconnect]
  )

  const renameTab = useCallback((key: string, title: string) => {
    setTabs((prev) => prev.map((t) => (t.key === key ? { ...t, title } : t)))
  }, [])

  const reorderTabsByKey = useCallback((fromKey: string, toKey: string) => {
    setTabs((prev) => reorderTabs(prev, fromKey, toKey))
  }, [])

  const setWorkspace = useCallback((key: string, tool: WorkspaceTool) => {
    setTabs((prev) => setTabWorkspace(prev, key, tool))
  }, [])

  const toggleSftp = useCallback((key: string) => {
    setTabs((prev) => toggleSftpOnTab(prev, key))
  }, [])

  const detachTab = useCallback(
    async (key: string) => {
      const tab = tabsRef.current.find((t) => t.key === key)
      if (!tab) return
      const check = canDetachTab(tab)
      if (!check.ok) {
        if (check.reason === 'multi-pane') {
          alert('Открепление пока поддерживается только для вкладки с одной панелью.')
        }
        return
      }
      const leaf = check.leaf

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
          onRestoredSshTabs()
        }
        layoutReadyRef.current = true
        return
      }
      if (s.openLocalOnStart) openLocalTab()
      layoutReadyRef.current = true
    })()
  }, [auxRestore, onRestoredSshTabs, openLocalTab, setServers])

  useEffect(() => {
    if (!layoutReadyRef.current) return
    const id = setTimeout(() => {
      void window.api.layout.set(tabsForLayoutPersist(tabs))
    }, 600)
    return () => clearTimeout(id)
  }, [tabs])

  useEffect(() => {
    const lookup = bindingLookup(settingsRef.current)
    const cycleTab = (dir: 1 | -1): void => {
      const next = nextTabKey(tabsRef.current, activeKeyRef.current, dir)
      if (next) setActiveKey(next)
    }
    const cyclePane = (dir: 1 | -1): void => {
      const key = activeKeyRef.current
      if (!key) return
      const tab = tabsRef.current.find((t) => t.key === key)
      if (!tab) return
      const next = nextActivePaneId(tab, dir)
      if (next) focusPane(key, next)
    }
    const onKey = (e: KeyboardEvent): void => {
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
        case 'command-palette':
          onCommandPalette()
          break
        case 'new-terminal':
          openLocalTab()
          break
        case 'close-tab':
          if (key) closeTab(key)
          break
        case 'next-tab':
          cycleTab(1)
          break
        case 'prev-tab':
          cycleTab(-1)
          break
        case 'split-right':
          if (key && isTerm) splitPane(key, 'row', { kind: 'local' })
          break
        case 'split-down':
          if (key && isTerm) splitPane(key, 'col', { kind: 'local' })
          break
        case 'close-pane':
          if (key && isTerm && tab) closePane(key, tab.activePaneId)
          break
        case 'focus-next-pane':
          cyclePane(1)
          break
        case 'focus-prev-pane':
          cyclePane(-1)
          break
        case 'toggle-sftp':
          if (key && isTerm) toggleSftp(key)
          break
        case 'toggle-broadcast':
          setBroadcast((b) => !b)
          break
        case 'open-settings':
          onOpenSettings()
          break
        default:
          handled = false
      }
      if (handled) {
        e.preventDefault()
        e.stopImmediatePropagation()
      }
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [
    settings.keybindings,
    openLocalTab,
    closeTab,
    splitPane,
    closePane,
    toggleSftp,
    focusPane,
    onCommandPalette,
    onOpenSettings
  ])

  const connectedSessions = useMemo(
    () =>
      tabs.flatMap((t) =>
        allLeaves(t.root)
          .filter((l) => l.kind === 'ssh' && l.status === 'connected' && l.sessionId)
          .map((l) => ({ sessionId: l.sessionId!, title: `${t.title} — ${l.title}` }))
      ),
    [tabs]
  )

  return {
    tabs,
    activeKey,
    setActiveKey,
    broadcast,
    setBroadcast,
    broadcastTargetCount,
    reconnect,
    openServerTab,
    openLocalTab,
    openEditorTab,
    closeTab,
    renameTab,
    reorderTabsByKey,
    splitPane,
    focusPane,
    closePane,
    resizeSplit,
    handleReady,
    handleFail,
    broadcastInput,
    setWorkspace,
    toggleSftp,
    detachTab,
    setEditorDirty,
    connectedSessions
  }
}
