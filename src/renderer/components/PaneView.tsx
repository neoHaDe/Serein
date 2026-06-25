import type { MouseEvent as ReactMouseEvent } from 'react'
import type { PaneNode } from '../paneTree'
import { TerminalView } from './TerminalView'

interface Props {
  node: PaneNode
  activePaneId: string
  /** Вкладка видима на экране. */
  tabActive: boolean
  canClose: boolean
  onFocusPane: (paneId: string) => void
  onReady: (paneId: string, sessionId: string) => void
  onInput: (fromSessionId: string, data: string) => void
  onClosePane: (paneId: string) => void
  onReconnect: (paneId: string) => void
  onResizeSplit: (splitId: string, sizes: [number, number]) => void
}

export function PaneView(props: Props): JSX.Element {
  const { node } = props

  if (node.type === 'leaf') {
    const isActive = node.id === props.activePaneId
    return (
      <div
        className={'pane' + (props.tabActive && isActive ? ' pane-active' : '')}
        onMouseDown={() => props.onFocusPane(node.id)}
      >
        {props.canClose && (
          <button
            className="pane-close"
            title="Закрыть панель"
            onClick={(e) => {
              e.stopPropagation()
              props.onClosePane(node.id)
            }}
          >
            ✕
          </button>
        )}
        <TerminalView
          key={`${node.id}:${node.gen}`}
          instanceKey={`${node.id}:${node.gen}`}
          paneId={node.id}
          kind={node.kind}
          serverId={node.serverId}
          active={props.tabActive}
          focused={props.tabActive && isActive}
          onReady={props.onReady}
          onInput={props.onInput}
        />
        {(node.status === 'closed' || node.status === 'error') && (
          <div className="reconnect-bar">
            <span>{node.statusMsg || 'Соединение закрыто'}</span>
            <button className="primary" onClick={() => props.onReconnect(node.id)}>
              ⟳ Переподключиться
            </button>
          </div>
        )}
      </div>
    )
  }

  // split-узел
  const isRow = node.dir === 'row'
  const startResize = (e: ReactMouseEvent): void => {
    e.preventDefault()
    const container = (e.currentTarget as HTMLElement).parentElement
    if (!container) return
    const rect = container.getBoundingClientRect()
    const onMove = (ev: MouseEvent): void => {
      const pos = isRow ? (ev.clientX - rect.left) / rect.width : (ev.clientY - rect.top) / rect.height
      const pct = Math.max(10, Math.min(90, pos * 100))
      props.onResizeSplit(node.id, [pct, 100 - pct])
    }
    const onUp = (): void => {
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
    }
    document.body.style.cursor = isRow ? 'col-resize' : 'row-resize'
    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp)
  }

  return (
    <div className={'pane-split ' + node.dir}>
      <div className="pane-child" style={{ flexGrow: node.sizes[0] }}>
        <PaneView {...props} node={node.children[0]} />
      </div>
      <div className={'pane-divider ' + node.dir} onMouseDown={startResize} />
      <div className="pane-child" style={{ flexGrow: node.sizes[1] }}>
        <PaneView {...props} node={node.children[1]} />
      </div>
    </div>
  )
}
