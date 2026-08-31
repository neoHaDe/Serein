import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { MouseEvent as ReactMouseEvent } from 'react'
import type { ServerConfig } from '../shared/types'
import { aggregateServerStatuses } from './serverStatus'
import { Sidebar } from './components/Sidebar'
import { TabBar } from './components/TabBar'
import { PaneView } from './components/PaneView'
import { useServerPrompts } from './hooks/useServerPrompts'
import { useAuxRestore } from './hooks/useAuxRestore'
import { useOperations } from './hooks/useOperations'
import { useTabs } from './hooks/useTabs'
import { ServerWorkspace } from './components/ServerWorkspace'
import { ServerForm } from './components/ServerForm'
import { SettingsModal } from './components/SettingsModal'
import { KiModal } from './components/KiModal'
import { HostKeyModal } from './components/HostKeyModal'
import { KeyGenModal } from './components/KeyGenModal'
import { GroupsModal } from './components/GroupsModal'
import { StatusBar } from './components/StatusBar'
import { CodeEditor } from './components/CodeEditor'
import { CommandPalette } from './components/CommandPalette'
import { useSettings } from './SettingsContext'
import { applyUiTheme } from './themes'
import { useWindowSnap } from './windowSnap'
import { isWindowsPlatform } from './platform'
import { buildPaletteItems } from './paletteItems'
import { flushAuxPersist, listenAuxGeoEvents } from './auxLayout'
import { allLeaves, findLeaf, type PaneLeaf } from './paneTree'
import { sshLeafForTools } from './tabs'
import { MultiExecModal } from './components/MultiExecModal'
import { ToolsModal } from './components/ToolsModal'

export default function App(): JSX.Element {
  const [editing, setEditing] = useState<ServerConfig | null | undefined>(undefined)
  const [showSettings, setShowSettings] = useState(false)
  const [showKeyGen, setShowKeyGen] = useState(false)
  const [showTools, setShowTools] = useState(false)
  const [showGroups, setShowGroups] = useState(false)
  const [sftpWidth, setSftpWidth] = useState(380)
  const [sidebarWidth, setSidebarWidth] = useState(270)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const prompts = useServerPrompts()
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [multiExec, setMultiExec] = useState(false)
  const [showPuttyImport, setShowPuttyImport] = useState(true)
  const { settings, update } = useSettings()
  useWindowSnap()

  const auxRestore = useAuxRestore()
  const persistSettingInit = useRef(false)
  const settingsRef = useRef(settings)
  settingsRef.current = settings

  const onSavedServer = useCallback(() => setEditing(undefined), [])

  const ops = useOperations(settings, update, onSavedServer)

  const tabsApi = useTabs({
    servers: ops.servers,
    setServers: ops.setServers,
    settings,
    auxRestore,
    onServerTabOpen: () => setSidebarCollapsed(true),
    onRestoredSshTabs: () => setSidebarCollapsed(true),
    onCommandPalette: () => setPaletteOpen((v) => !v),
    onOpenSettings: () => setShowSettings(true)
  })

  useEffect(() => listenAuxGeoEvents(), [])

  useEffect(() => {
    void isWindowsPlatform().then(setShowPuttyImport)
  }, [])

  useEffect(() => {
    const t = window.setTimeout(() => window.dispatchEvent(new Event('resize')), 100)
    return () => window.clearTimeout(t)
  }, [tabsApi.activeKey, sftpWidth, tabsApi.tabs.length])

  useEffect(() => {
    if (!persistSettingInit.current) {
      persistSettingInit.current = true
      return
    }
    if (settings.restoreAuxOnStart) flushAuxPersist()
  }, [settings.restoreAuxOnStart])

  useEffect(() => {
    if (settings.sidebarWidth) setSidebarWidth(settings.sidebarWidth)
    if (settings.sftpWidth) setSftpWidth(settings.sftpWidth)
  }, [settings.sidebarWidth, settings.sftpWidth])

  useEffect(() => {
    applyUiTheme(settings.theme)
  }, [settings.theme])

  useEffect(() => {
    document.documentElement.dataset.density = settings.density ?? 'comfortable'
  }, [settings.density])

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

  const serverStatuses = useMemo(() => aggregateServerStatuses(tabsApi.tabs), [tabsApi.tabs])

  const paletteItems = useMemo(
    () =>
      buildPaletteItems(ops.servers, tabsApi.tabs, tabsApi.activeKey, {
        openServer: tabsApi.openServerTab,
        openLocal: tabsApi.openLocalTab,
        openSettings: () => setShowSettings(true),
        openKeyGen: () => setShowKeyGen(true),
        openTools: () => setShowTools(true),
        newServer: () => setEditing(null),
        setWorkspace: tabsApi.setWorkspace,
        focusTab: tabsApi.setActiveKey
      }),
    [ops.servers, tabsApi]
  )

  return (
    <div className="app">
      <Sidebar
        servers={ops.servers}
        onConnect={tabsApi.openServerTab}
        onOpenLocal={tabsApi.openLocalTab}
        onNew={() => setEditing(null)}
        onEdit={(s) => setEditing(s)}
        onDelete={ops.deleteServer}
        onOpenSettings={() => setShowSettings(true)}
        onOpenKeyGen={() => setShowKeyGen(true)}
        onImport={ops.importServers}
        showPuttyImport={showPuttyImport}
        width={sidebarWidth}
        collapsed={sidebarCollapsed}
        onToggleCollapse={() => setSidebarCollapsed((v) => !v)}
        statuses={serverStatuses}
        groupOrder={ops.allGroups}
        collapsedGroups={ops.collapsedGroups}
        onToggleGroup={ops.toggleGroup}
        onNewGroup={ops.createGroup}
        onOpenGroups={() => setShowGroups(true)}
        onMultiExec={() => setMultiExec(true)}
        onPatch={(id, patch) => void ops.patchServer(id, patch)}
        onDropServer={(id, group, index) => void ops.dropServer(id, group, index)}
        onDropGroup={ops.dropGroup}
      />

      {!sidebarCollapsed && <div className="sidebar-resizer" onMouseDown={startSidebarResize} />}

      <div className="workspace">
        <TabBar
          tabs={tabsApi.tabs}
          activeKey={tabsApi.activeKey}
          servers={ops.servers}
          onSelect={tabsApi.setActiveKey}
          onClose={tabsApi.closeTab}
          onNewLocal={tabsApi.openLocalTab}
          onToggleSftp={tabsApi.toggleSftp}
          onSetWorkspace={tabsApi.setWorkspace}
          onDetachTab={(key) => void tabsApi.detachTab(key)}
          onRename={tabsApi.renameTab}
          onReorder={tabsApi.reorderTabsByKey}
          onSplit={tabsApi.splitPane}
          broadcast={tabsApi.broadcast}
          onToggleBroadcast={() => tabsApi.setBroadcast((b) => !b)}
        />

        <div className="terminals">
          {tabsApi.tabs.length === 0 && ops.servers.length === 0 && (
            <div className="empty-state">
              <h1>Serein</h1>
              <p>
                Серверов пока нет. Быстрее всего — забрать те, что уже
                <br />
                настроены на этой машине.
              </p>
              <div className="welcome-actions">
                <button onClick={() => void ops.importServers('ssh')}>Импорт из ~/.ssh/config</button>
                {showPuttyImport && (
                  <>
                    <button onClick={() => void ops.importServers('putty')}>Импорт сессий PuTTY</button>
                    <button onClick={() => void ops.importServers('mobaxterm')}>Импорт MobaXterm</button>
                    <button onClick={() => void ops.importServers('xshell')}>Импорт XShell</button>
                    <button onClick={() => void ops.importServers('securecrt')}>Импорт SecureCRT</button>
                  </>
                )}
                <button onClick={() => setEditing(null)}>Добавить сервер вручную</button>
              </div>
              <p className="welcome-hint">Локальный терминал работает и без настройки.</p>
              <button className="ghost" onClick={tabsApi.openLocalTab}>
                Открыть локальный терминал
              </button>
            </div>
          )}

          {tabsApi.tabs.length === 0 && ops.servers.length > 0 && (
            <div className="empty-state">
              <h1>Serein</h1>
              <p>
                Выберите сервер слева для подключения по SSH
                <br />
                или откройте локальный терминал.
              </p>
              <button onClick={tabsApi.openLocalTab}>Открыть локальный терминал</button>
            </div>
          )}

          {tabsApi.tabs.map((tab) => {
            const isActive = tab.key === tabsApi.activeKey
            if (tab.kind === 'editor' && tab.editor) {
              return (
                <div key={tab.key} className="terminal-slot" style={{ display: isActive ? 'flex' : 'none' }}>
                  <CodeEditor
                    sessionId={tab.editor.sessionId}
                    remotePath={tab.editor.remotePath}
                    fileName={tab.title}
                    active={isActive}
                    onDirtyChange={(d) => tabsApi.setEditorDirty(tab.key, d)}
                  />
                </div>
              )
            }
            const sshLeaf = sshLeafForTools(tab)
            const showRail = !!sshLeaf
            const tool = showRail ? tab.workspace : 'terminal'
            const sessionId =
              sshLeaf && sshLeaf.status === 'connected' ? sshLeaf.sessionId : undefined
            const goTerminal = (): void => tabsApi.setWorkspace(tab.key, 'terminal')
            return (
              <div
                key={tab.key}
                className="terminal-slot"
                style={{ display: isActive ? 'flex' : 'none' }}
              >
                <ServerWorkspace
                  leaf={showRail && sshLeaf ? sshLeaf : undefined}
                  server={ops.servers.find((srv) => srv.id === sshLeaf?.serverId)}
                  title={tab.title}
                  panelTitle={tab.title}
                  tool={tool}
                  onSelectTool={(t) => tabsApi.setWorkspace(tab.key, t)}
                  onReconnect={() => sshLeaf && tabsApi.reconnect.now(tab.key, sshLeaf.id)}
                  onEditServer={() => {
                    const srv = ops.servers.find((x) => x.id === sshLeaf?.serverId)
                    if (srv) setEditing(srv)
                  }}
                  sessionId={sessionId}
                  serverId={sshLeaf?.serverId}
                  sftpOpen={tab.sftpOpen}
                  sftpWidth={sftpWidth}
                  onSftpClose={() => tabsApi.toggleSftp(tab.key)}
                  onSftpResizeStart={startSftpResize}
                  onOpenInEditor={sessionId ? (rp) => tabsApi.openEditorTab(sessionId, rp) : undefined}
                  onDetached={goTerminal}
                  terminal={
                    <PaneView
                      node={tab.root}
                      activePaneId={tab.activePaneId}
                      tabActive={tab.key === tabsApi.activeKey}
                      canClose={allLeaves(tab.root).length > 1}
                      onFocusPane={(pid) => tabsApi.focusPane(tab.key, pid)}
                      onReady={tabsApi.handleReady}
                      onFail={tabsApi.handleFail}
                      onInput={tabsApi.broadcastInput}
                      onClosePane={(pid) => tabsApi.closePane(tab.key, pid)}
                      onReconnect={(pid) => tabsApi.reconnect.now(tab.key, pid)}
                      onCancelReconnect={(pid) => tabsApi.reconnect.cancel(tab.key, pid)}
                      onResizeSplit={(sid, sizes) => tabsApi.resizeSplit(tab.key, sid, sizes)}
                    />
                  }
                />
              </div>
            )
          })}
        </div>

        {(() => {
          const activeTab = tabsApi.tabs.find((t) => t.key === tabsApi.activeKey)
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
          const srv = ops.servers.find((s) => s.id === activeLeaf?.serverId)
          return (
            <StatusBar
              leaf={activeLeaf}
              server={srv}
              broadcast={tabsApi.broadcast}
              broadcastTargets={tabsApi.broadcastTargetCount}
            />
          )
        })()}
      </div>

      {editing !== undefined && (
        <ServerForm
          initial={editing}
          servers={ops.servers}
          onCancel={() => setEditing(undefined)}
          onSave={ops.saveServer}
        />
      )}

      {paletteOpen && <CommandPalette items={paletteItems} onClose={() => setPaletteOpen(false)} />}

      {showSettings && <SettingsModal onClose={() => setShowSettings(false)} />}

      {multiExec && <MultiExecModal servers={ops.servers} onClose={() => setMultiExec(false)} />}

      {showGroups && (
        <GroupsModal
          groups={ops.allGroups}
          servers={ops.servers}
          onClose={() => setShowGroups(false)}
          onRename={(from, to) => void ops.renameGroup(from, to)}
          onDelete={(g) => void ops.deleteGroup(g)}
          onCreate={(name) => ops.saveGroupOrder([...(settingsRef.current.groupOrder ?? []), name])}
          onAssign={(id, group) => void ops.dropServer(id, group)}
          onMove={ops.moveGroup}
        />
      )}

      {showKeyGen && (
        <KeyGenModal connectedSessions={tabsApi.connectedSessions} onClose={() => setShowKeyGen(false)} />
      )}

      {showTools && <ToolsModal onClose={() => setShowTools(false)} />}

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
