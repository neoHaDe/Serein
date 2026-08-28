import { useState } from 'react'
import type { ServerConfig, WorkspaceTool } from '../../shared/types'
import type { PaneLeaf } from '../paneTree'
import type { IconName } from './Icon'
import { Icon } from './Icon'

const ITEMS: { id: WorkspaceTool; label: string; icon: IconName }[] = [
  { id: 'terminal', label: 'Terminal', icon: 'terminal' },
  { id: 'docker', label: 'Docker', icon: 'docker' },
  { id: 'logs', label: 'Logs', icon: 'logs' },
  { id: 'processes', label: 'Processes', icon: 'list' },
  { id: 'services', label: 'Services', icon: 'settings' },
  { id: 'tunnels', label: 'Tunnels', icon: 'tunnel' }
]

const STATUS_LABEL: Record<PaneLeaf['status'], string> = {
  connecting: 'Подключение…',
  connected: 'Подключён',
  reconnecting: 'Переподключение…',
  closed: 'Нет связи',
  error: 'Ошибка'
}

const STATUS_COLOR: Record<PaneLeaf['status'], string> = {
  connecting: '#e0af68',
  connected: '#9ece6a',
  reconnecting: '#e0af68',
  closed: '#565f89',
  error: '#f7768e'
}

interface Props {
  title: string
  leaf: PaneLeaf
  server: ServerConfig | undefined
  tool: WorkspaceTool
  onSelect: (tool: WorkspaceTool) => void
  onReconnect: () => void
  onEditServer: () => void
}

export function WorkspaceRail({
  title,
  leaf,
  server,
  tool,
  onSelect,
  onReconnect,
  onEditServer
}: Props): JSX.Element {
  const [copied, setCopied] = useState(false)
  const connected = leaf.status === 'connected' && !!leaf.sessionId
  const host = server ? `${server.username}@${server.host}` : ''

  const copyHost = async (): Promise<void> => {
    if (!host) return
    await window.api.clipboard.write(host)
    setCopied(true)
    setTimeout(() => setCopied(false), 1200)
  }

  return (
    <nav className="ws-rail" aria-label="Инструменты сервера">
      <div className="ws-rail-head">
        <div className="ws-rail-name" title={leaf.statusMsg || title}>
          <span className="status-dot" style={{ background: STATUS_COLOR[leaf.status] }} />
          <span className="ws-rail-title">{title}</span>
        </div>
        <div className="ws-rail-status">{STATUS_LABEL[leaf.status]}</div>
        {host && <div className="ws-rail-host">{host}</div>}
      </div>
      <div className="ws-nav">
        {ITEMS.map((item) => {
          const disabled = item.id !== 'terminal' && !connected
          return (
            <button
              key={item.id}
              type="button"
              className={'ws-nav-item' + (tool === item.id ? ' active' : '')}
              disabled={disabled}
              title={disabled ? 'Сначала подключитесь' : item.label}
              onClick={() => onSelect(item.id)}
            >
              <Icon name={item.icon} size={14} />
              {item.label}
            </button>
          )
        })}
      </div>
      <div className="ws-quick">
        <button
          type="button"
          className="ws-quick-btn"
          disabled={leaf.status === 'connecting' || leaf.status === 'reconnecting'}
          onClick={onReconnect}
        >
          Переподключить
        </button>
        {server && (
          <button type="button" className="ws-quick-btn" onClick={onEditServer}>
            Редактировать
          </button>
        )}
        {host && (
          <button type="button" className="ws-quick-btn" onClick={() => void copyHost()}>
            {copied ? 'Скопировано' : 'Копировать user@host'}
          </button>
        )}
      </div>
    </nav>
  )
}
