import { useEffect, useState } from 'react'
import { MonitorMetrics } from './MonitorMetrics'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'
import type { WorkspaceTool } from '../../shared/types'

/** Как называем систему в интерфейсе: коротко и на языке пользователя. */
const PLATFORM_LABEL: Record<string, string> = {
  linux: 'Linux',
  busybox: 'BusyBox',
  windows: 'Windows'
}

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
  // Систему определяет бэкенд одним зондом на сессию. Показываем её здесь, потому что от
  // неё зависит, что вообще умеют остальные панели: на Windows нет ни `ps`, ни systemd.
  const [platform, setPlatform] = useState<{ kind: string; version: string } | null>(null)
  useEffect(() => {
    let alive = true
    void window.api.workspace
      .platform(sessionId)
      .then((p) => alive && setPlatform(p))
      .catch(() => undefined)
    return () => {
      alive = false
    }
  }, [sessionId])

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
          {platform && platform.kind !== 'unknown' && (
            <span className="ws-platform" title={platform.version || undefined}>
              {PLATFORM_LABEL[platform.kind] ?? platform.kind}
            </span>
          )}
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
