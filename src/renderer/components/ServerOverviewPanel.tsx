import { MonitorMetrics } from './MonitorMetrics'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'
import type { WorkspaceTool } from '../../shared/types'

interface Props {
  sessionId: string
  panelTitle?: string
  onDetached?: () => void
  onGoTool?: (tool: WorkspaceTool) => void
  fill?: boolean
}

export function ServerOverviewPanel({
  sessionId,
  panelTitle,
  onDetached,
  onGoTool,
  fill
}: Props): JSX.Element {
  const detach = async (): Promise<void> => {
    if (!panelTitle) return
    await openDetachedWorkspace({ tool: 'overview', sessionId, title: panelTitle })
    onDetached?.()
  }

  return (
    <div className={'ws-panel srv-overview' + (fill ? ' fill' : '')}>
      <div className="ws-head">
        <span className="ws-head-title">
          <Icon name="monitor" size={15} /> Обзор сервера
        </span>
        <div style={{ display: 'flex', gap: 6 }}>
          {panelTitle && onDetached && <WsDetachButton onClick={detach} />}
        </div>
      </div>
      <MonitorMetrics
        sessionId={sessionId}
        variant="dashboard"
        showOverviewCards
        onGoTool={onGoTool}
      />
    </div>
  )
}
