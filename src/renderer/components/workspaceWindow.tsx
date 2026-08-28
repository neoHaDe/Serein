import { useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { ServerConfig, WorkspaceTool } from '../../shared/types'
import { parseWorkspaceTool } from '../../shared/types'
import { applyUiTheme } from '../themes'
import { useWindowSnap } from '../windowSnap'
import { openAuxWindow, sanitizeWindowLabel } from '../auxWindows'
import { reattachWorkspace } from '../reattach'
import { AppChrome, markAuxWindow } from './WindowChrome'
import { AuxReattachButton } from './AuxReattachButton'
import { DockerPanel } from './DockerPanel'
import { HostLogsPanel } from './HostLogsPanel'
import { ProcessPanel } from './ProcessPanel'
import { ServicePanel } from './ServicePanel'
import { TunnelPanel } from './TunnelPanel'

export type DetachableWorkspaceTool = Exclude<WorkspaceTool, 'terminal'>

const TOOL_TITLE: Record<DetachableWorkspaceTool, string> = {
  docker: 'Docker',
  logs: 'Logs',
  processes: 'Processes',
  services: 'Services',
  tunnels: 'Tunnels'
}

export async function openDetachedWorkspace(opts: {
  tool: DetachableWorkspaceTool
  sessionId: string
  serverId?: string
  title: string
}): Promise<void> {
  const label = `ws-${opts.tool}-${sanitizeWindowLabel(opts.sessionId)}`
  await openAuxWindow({
    label,
    query: {
      workspace: '1',
      tool: opts.tool,
      sessionId: opts.sessionId,
      serverId: opts.serverId ?? '',
      title: opts.title
    },
    title: `${opts.title} - ${TOOL_TITLE[opts.tool]}`,
    width: 760,
    height: 560
  })
}

function panelForTool(
  tool: DetachableWorkspaceTool,
  sessionId: string,
  server: ServerConfig | undefined
): JSX.Element {
  switch (tool) {
    case 'docker':
      return <DockerPanel sessionId={sessionId} serverId={server?.id} docked fill onClose={() => void getCurrentWindow().close()} />
    case 'logs':
      return <HostLogsPanel sessionId={sessionId} fill />
    case 'processes':
      return <ProcessPanel sessionId={sessionId} fill />
    case 'services':
      return <ServicePanel sessionId={sessionId} fill />
    case 'tunnels':
      return (
        <TunnelPanel
          sessionId={sessionId}
          server={server}
          docked
          fill
          onClose={() => void getCurrentWindow().close()}
          onEditServer={() => {}}
        />
      )
  }
}

export function WorkspaceWindow(): JSX.Element {
  const q = new URLSearchParams(window.location.search)
  const sessionId = q.get('sessionId') ?? ''
  const serverId = q.get('serverId') ?? ''
  const title = q.get('title') ?? 'SSH'
  const tool = parseWorkspaceTool(q.get('tool')) as DetachableWorkspaceTool
  const [servers, setServers] = useState<ServerConfig[]>([])
  useWindowSnap()

  useEffect(() => {
    markAuxWindow()
    void window.api.settings.get().then((s) => {
      applyUiTheme(s.theme)
      document.documentElement.dataset.density = s.density ?? 'comfortable'
    })
    void window.api.servers.list().then(setServers)
  }, [])

  const server = servers.find((s) => s.id === serverId)
  const winTitle = `${title} - ${TOOL_TITLE[tool] ?? tool}`

  return (
    <div className="aux-workspace-root">
      <AppChrome
        title={winTitle}
        actions={
          <AuxReattachButton
            onClick={() =>
              reattachWorkspace({
                sessionId,
                serverId: serverId || undefined,
                title,
                tool
              })
            }
          />
        }
      />
      <div className="aux-workspace-body">{panelForTool(tool, sessionId, server)}</div>
    </div>
  )
}