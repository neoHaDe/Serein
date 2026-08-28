import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import type { MouseEvent as ReactMouseEvent } from 'react'
import type { ServerConfig, WorkspaceTool } from '../../shared/types'
import { parseWorkspaceTool } from '../../shared/types'
import type { PaneLeaf } from '../paneTree'
import { applyUiTheme } from '../themes'
import { useWindowSnap } from '../windowSnap'
import { openAuxWindow, sanitizeWindowLabel } from '../auxWindows'
import { clearDetachedMark, markSessionDetached, shouldPreserveSession } from '../detachedSessions'
import { reattachTab } from '../reattach'
import { AppChrome, markAuxWindow } from './WindowChrome'
import { WorkspaceRail } from './WorkspaceRail'
import { TerminalView } from './TerminalView'
import { SftpPanel } from './SftpPanel'
import { DockerPanel } from './DockerPanel'
import { HostLogsPanel } from './HostLogsPanel'
import { ProcessPanel } from './ProcessPanel'
import { ServicePanel } from './ServicePanel'
import { TunnelPanel } from './TunnelPanel'
import { AuxReattachButton } from './AuxReattachButton'
import { Icon } from './Icon'
import type { PaneKind } from '../../shared/types'

const paneStack: CSSProperties = {
  minWidth: 0,
  minHeight: 0,
  display: 'flex',
  flexDirection: 'column',
  overflow: 'hidden'
}

export async function openDetachedTabWindow(opts: {
  sessionId: string
  serverId?: string
  title: string
  workspace: WorkspaceTool
  sftpOpen: boolean
  kind: PaneKind
}): Promise<void> {
  const label = `tab-${sanitizeWindowLabel(opts.sessionId)}`
  await openAuxWindow({
    label,
    query: {
      detachedTab: '1',
      sessionId: opts.sessionId,
      serverId: opts.serverId ?? '',
      title: opts.title,
      workspace: opts.workspace,
      sftpOpen: opts.sftpOpen ? '1' : '0',
      kind: opts.kind
    },
    title: opts.title,
    width: 980,
    height: 640,
    minWidth: 520,
    minHeight: 360
  })
}

export function DetachedTabWindow(): JSX.Element {
  const q = new URLSearchParams(window.location.search)
  const sessionId = q.get('sessionId') ?? ''
  const serverId = q.get('serverId') ?? ''
  const title = q.get('title') ?? 'Terminal'
  const kindParam = q.get('kind')
  const kind: PaneKind =
    kindParam === 'local' ? 'local' : kindParam === 'serial' ? 'serial' : 'ssh'
  const initialTool = parseWorkspaceTool(q.get('workspace'))
  const initialSftp = q.get('sftpOpen') === '1'
  const reattachingRef = useRef(false)
  const slotRef = useRef<HTMLDivElement>(null)

  const [servers, setServers] = useState<ServerConfig[]>([])
  const [tool, setTool] = useState<WorkspaceTool>(initialTool)
  const [sftpOpen, setSftpOpen] = useState(initialSftp)
  const [sftpWidth, setSftpWidth] = useState(380)
  useWindowSnap()

  const server = useMemo(() => servers.find((s) => s.id === serverId), [servers, serverId])

  const leaf: PaneLeaf = useMemo(
    () => ({
      type: 'leaf',
      id: 'detached-pane',
      kind,
      serverId: serverId || undefined,
      title,
      sessionId,
      status: 'connected',
      gen: 0
    }),
    [kind, serverId, title, sessionId]
  )

  useEffect(() => {
    markAuxWindow()
    void window.api.settings.get().then((s) => {
      applyUiTheme(s.theme)
      document.documentElement.dataset.density = s.density ?? 'comfortable'
      if (s.sftpWidth) setSftpWidth(s.sftpWidth)
    })
    void window.api.servers.list().then(setServers)
    return () => {
      if (!sessionId || reattachingRef.current) return
      if (shouldPreserveSession(sessionId)) return
      clearDetachedMark(sessionId)
      void window.api.session.close(sessionId)
    }
  }, [sessionId])

  const startSftpResize = useCallback((e: ReactMouseEvent) => {
    e.preventDefault()
    const slot = slotRef.current
    const onMove = (ev: MouseEvent): void => {
      const right = slot?.getBoundingClientRect().right ?? window.innerWidth
      setSftpWidth(Math.max(260, Math.min(820, right - ev.clientX)))
    }
    const onUp = (): void => {
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
    }
    document.body.style.cursor = 'col-resize'
    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp)
  }, [])

  const showRail = kind === 'ssh' && !!serverId
  const canSftp = kind === 'ssh' && !!sessionId
  const showSftp = tool === 'terminal' && sftpOpen && !!sessionId

  const bodyStyle: CSSProperties = {
    flex: 1,
    minHeight: 0,
    minWidth: 0,
    overflow: 'hidden',
    display: 'grid',
    gridTemplateRows: 'minmax(0, 1fr)',
    gridTemplateColumns: showRail ? 'auto minmax(0, 1fr)' : 'minmax(0, 1fr)'
  }

  const slotStyle: CSSProperties = {
    minWidth: 0,
    minHeight: 0,
    overflow: 'hidden',
    display: 'grid',
    gridTemplateRows: 'minmax(0, 1fr)',
    gridTemplateColumns: showSftp ? `minmax(0, 1fr) 5px ${sftpWidth}px` : 'minmax(0, 1fr)'
  }

  const doReattach = (): void => {
    reattachingRef.current = true
    markSessionDetached(sessionId)
    void reattachTab({
      sessionId,
      serverId: serverId || undefined,
      title,
      workspace: tool,
      sftpOpen,
      kind
    })
  }

  return (
    <div className="detached-tab-root">
      <AppChrome
        title={title}
        actions={
          <>
            {canSftp && (
              <button
                type="button"
                className={'mini aux-chrome-tool' + (sftpOpen ? ' on' : '')}
                title="Файловый менеджер (SFTP)"
                onClick={() => {
                  setSftpOpen((v) => !v)
                  setTool('terminal')
                }}
              >
                <Icon name="folder" size={14} />
              </button>
            )}
            <AuxReattachButton onClick={doReattach} />
          </>
        }
      />
      <div className="detached-tab-body" style={bodyStyle}>
        {showRail && (
          <WorkspaceRail
            title={title}
            leaf={leaf}
            server={server}
            tool={tool}
            onSelect={setTool}
            onReconnect={() => {}}
            onEditServer={() => {}}
          />
        )}
        <div ref={slotRef} className="detached-tab-slot" style={slotStyle}>
          {tool === 'terminal' ? (
            <>
              <div className="pane-area" style={{ ...paneStack, gridColumn: 1 }}>
                <div className="pane pane-active" style={paneStack}>
                  <TerminalView
                    key={`detached:${sessionId}`}
                    instanceKey={`detached:${sessionId}`}
                    paneId="detached-pane"
                    kind={kind}
                    serverId={serverId || undefined}
                    attachSessionId={sessionId}
                    active
                    focused
                    onReady={() => {}}
                  />
                </div>
              </div>
              {showSftp && (
                <>
                  <div
                    className="sftp-resizer"
                    style={{ gridColumn: 2, width: 5, minHeight: 0 }}
                    onMouseDown={startSftpResize}
                  />
                  <div style={{ ...paneStack, gridColumn: 3 }}>
                    <SftpPanel
                      sessionId={sessionId}
                      serverId={serverId || undefined}
                      width={sftpWidth}
                      closing={false}
                      fill
                      onClose={() => setSftpOpen(false)}
                    />
                  </div>
                </>
              )}
            </>
          ) : (
            <div className="ws-body" style={paneStack}>
              {tool === 'docker' && (
                <DockerPanel sessionId={sessionId} serverId={serverId || undefined} docked onClose={() => setTool('terminal')} />
              )}
              {tool === 'logs' && <HostLogsPanel sessionId={sessionId} />}
              {tool === 'processes' && <ProcessPanel sessionId={sessionId} />}
              {tool === 'services' && <ServicePanel sessionId={sessionId} />}
              {tool === 'tunnels' && (
                <TunnelPanel
                  sessionId={sessionId}
                  server={server}
                  docked
                  onClose={() => setTool('terminal')}
                  onEditServer={() => {}}
                />
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
