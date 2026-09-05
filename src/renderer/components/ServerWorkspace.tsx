import type { CSSProperties, MouseEvent as ReactMouseEvent, ReactNode } from 'react'
import type { ServerConfig, WorkspaceTool } from '../../shared/types'
import type { PaneLeaf } from '../paneTree'
import { WorkspaceRail } from './WorkspaceRail'
import { SftpPanel } from './SftpPanel'
import { DockerPanel } from './DockerPanel'
import { HostLogsPanel } from './HostLogsPanel'
import { ProcessPanel } from './ProcessPanel'
import { ServicePanel } from './ServicePanel'
import { TunnelPanel } from './TunnelPanel'
import { ServerOverviewPanel } from './ServerOverviewPanel'
import { VncPanel } from './VncPanel'
import { DatabasePanel } from './DatabasePanel'

/**
 * Рабочее пространство сервера: рельс инструментов, терминал, файлы и панели.
 *
 * Собрано в одно место потому, что раньше этого места было два — главное окно и
 * откреплённое рисовали один и тот же экран независимо друг от друга. Они разъехались, и
 * это стоило релиза 1.2.6: в главном окне терминал прятался стилем и оставался живым, а в
 * откреплённом размонтировался — и уносил с собой SSH-сессию при первом же переключении на
 * Docker. Снаружи это выглядело как «окно сломалось».
 *
 * Отличие между окнами ровно одно и оно осталось: в главном внутри терминальной области
 * дерево панелей со сплитами, в откреплённом — один терминал. Поэтому содержимое приходит
 * слотом `terminal`, а всё остальное — раскладка, правило «прятать, а не размонтировать»,
 * разделитель SFTP, набор панелей — живёт здесь в единственном экземпляре.
 */
export interface ServerWorkspaceProps {
  /** Панель, по которой рисуется рельс: статус, имя, кнопка переподключения. */
  leaf?: PaneLeaf
  server?: ServerConfig
  title: string
  tool: WorkspaceTool
  onSelectTool: (tool: WorkspaceTool) => void
  onReconnect: () => void
  onEditServer: () => void
  /**
   * Сессия для панелей Docker/логов/процессов. `undefined`, если соединения нет —
   * панели тогда не открываются, а не показывают пустоту от мёртвой сессии.
   */
  sessionId?: string
  serverId?: string
  /** Терминальная область: дерево панелей в главном окне, один терминал в откреплённом. */
  terminal: ReactNode
  sftpOpen: boolean
  sftpWidth: number
  onSftpClose: () => void
  onSftpResizeStart?: (e: ReactMouseEvent) => void
  onOpenInEditor?: (remotePath: string) => void
  /** Заголовок в шапке панели — в откреплённом окне он уже есть в рамке окна. */
  panelTitle?: string
  /** Откреплять панели можно только из главного окна: во втором это некуда. */
  onDetached?: () => void
}

const paneStack: CSSProperties = {
  minWidth: 0,
  minHeight: 0,
  display: 'flex',
  flexDirection: 'column',
  overflow: 'hidden'
}

export function ServerWorkspace(props: ServerWorkspaceProps): JSX.Element {
  const {
    leaf,
    server,
    title,
    tool,
    onSelectTool,
    onReconnect,
    onEditServer,
    sessionId,
    serverId,
    terminal,
    sftpOpen,
    sftpWidth,
    onSftpClose,
    onSftpResizeStart,
    onOpenInEditor,
    panelTitle,
    onDetached
  } = props

  const showRail = !!leaf
  const showSftp = tool === 'terminal' && sftpOpen && !!sessionId
  const goTerminal = (): void => onSelectTool('terminal')

  return (
    <>
      {showRail && leaf && (
        <WorkspaceRail
          title={title}
          leaf={leaf}
          server={server}
          tool={tool}
          onSelect={onSelectTool}
          onReconnect={onReconnect}
          onEditServer={onEditServer}
        />
      )}

      {/*
        Терминал прячем стилем и НЕ размонтируем. Размонтирование освобождает xterm вместе
        с сессией, а поход в Docker и обратно должен её пережить — вместе со скроллбэком.
        Это то самое решение, расхождение по которому и сломало откреплённое окно.
      */}
      <div className="pane-area" style={{ display: tool === 'terminal' ? 'flex' : 'none' }}>
        {terminal}
      </div>

      {showSftp && (
        <>
          <div className="sftp-resizer" onMouseDown={onSftpResizeStart} />
          <SftpPanel
            sessionId={sessionId!}
            serverId={serverId}
            width={sftpWidth}
            closing={false}
            onClose={onSftpClose}
            onOpenInEditor={onOpenInEditor}
          />
        </>
      )}

      {showRail && tool !== 'terminal' && (
        <div className="ws-body" style={paneStack}>
          {!sessionId && <div className="ws-waiting">Нет активного SSH-соединения</div>}
          {sessionId && tool === 'overview' && (
            <ServerOverviewPanel
              sessionId={sessionId}
              panelTitle={panelTitle}
              onDetached={onDetached}
              onGoTool={onSelectTool}
            />
          )}
          {sessionId && tool === 'docker' && (
            <DockerPanel
              sessionId={sessionId}
              serverId={serverId}
              panelTitle={panelTitle}
              docked
              onClose={goTerminal}
              onGoTerminal={goTerminal}
              onDetached={onDetached ?? goTerminal}
            />
          )}
          {sessionId && tool === 'logs' && (
            <HostLogsPanel sessionId={sessionId} panelTitle={panelTitle} onDetached={onDetached} />
          )}
          {sessionId && tool === 'processes' && (
            <ProcessPanel sessionId={sessionId} panelTitle={panelTitle} onDetached={onDetached} />
          )}
          {sessionId && tool === 'services' && (
            <ServicePanel sessionId={sessionId} panelTitle={panelTitle} onDetached={onDetached} />
          )}
          {sessionId && tool === 'databases' && (
            <DatabasePanel sessionId={sessionId} panelTitle={panelTitle} onDetached={onDetached} />
          )}
          {sessionId && tool === 'desktop' && (
            <VncPanel sessionId={sessionId} panelTitle={panelTitle} onDetached={onDetached} />
          )}
          {sessionId && tool === 'tunnels' && (
            <TunnelPanel
              sessionId={sessionId}
              server={server}
              panelTitle={panelTitle}
              docked
              onClose={goTerminal}
              onDetached={onDetached}
              onEditServer={onEditServer}
            />
          )}
        </div>
      )}
    </>
  )
}
