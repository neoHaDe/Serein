import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { MouseEvent as ReactMouseEvent } from 'react'
import type { ServerConfig, WorkspaceTool } from '../../shared/types'
import { parsePaneKind, parseWorkspaceTool } from '../../shared/types'
import type { PaneLeaf } from '../paneTree'
import { applyUiTheme } from '../themes'
import { useWindowSnap } from '../windowSnap'
import { openAuxWindow, sanitizeWindowLabel } from '../auxWindows'
import { takeOverSession } from '../detachedSessions'
import { reattachTab } from '../reattach'
import { AppChrome, markAuxWindow } from './WindowChrome'
import { TerminalView } from './TerminalView'
import { ServerWorkspace } from './ServerWorkspace'
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

/**
 * Метка окна открепления по идентификатору сессии.
 *
 * Экспортируется, чтобы главное окно могло передать владение сессией ещё ДО того, как
 * окно откроется. Считать эту метку во втором месте нельзя: разошлись бы — и владение
 * ушло бы в никуда, а сессию закрыл бы первый, кто размонтируется.
 */
export function detachedWindowLabel(sessionId: string): string {
  return `tab-${sanitizeWindowLabel(sessionId)}`
}

export async function openDetachedTabWindow(opts: {
  sessionId: string
  serverId?: string
  title: string
  workspace: WorkspaceTool
  sftpOpen: boolean
  kind: PaneKind
}): Promise<void> {
  const label = detachedWindowLabel(opts.sessionId)
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
  // Сессия в состоянии, а не константой: при переподключении она меняется,
  // и всё окно должно переехать на новую.
  const [sessionId, setSessionId] = useState(q.get('sessionId') ?? '')
  const serverId = q.get('serverId') ?? ''
  const title = q.get('title') ?? 'Terminal'
  const kindParam = q.get('kind')
  const kind: PaneKind = parsePaneKind(kindParam)
  const initialTool = parseWorkspaceTool(q.get('workspace'))
  const initialSftp = q.get('sftpOpen') === '1'
  const reattachingRef = useRef(false)
  const sessionRef = useRef(sessionId)
  sessionRef.current = sessionId
  const slotRef = useRef<HTMLDivElement>(null)

  const [servers, setServers] = useState<ServerConfig[]>([])
  const [tool, setTool] = useState<WorkspaceTool>(initialTool)
  const [sftpOpen, setSftpOpen] = useState(initialSftp)
  const [sftpWidth, setSftpWidth] = useState(380)
  /**
   * Живо ли соединение.
   *
   * Раньше здесь стояло жёсткое `'connected'`, и окно об обрыве не узнавало никогда:
   * заголовок продолжал показывать «Подключён», Docker и логи молча ничего не открывали
   * (сессии на той стороне уже нет), а возврат вкладки отдавал в главное окно мёртвый
   * идентификатор. Снаружи это выглядит как «окно сломалось».
   */
  const [status, setStatus] = useState<PaneLeaf['status']>('connected')
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
      status,
      gen: 0
    }),
    [kind, serverId, title, sessionId, status]
  )

  // Забираем сессию себе: закрыть её теперь может только окно-владелец, и запись об
  // этом одна на стороне Rust. Главное окно передаёт владение ещё при откреплении —
  // здесь мы лишь подтверждаем его для сессий, поднятых уже в этом окне
  // (переподключение), и на случай, если окно восстановили после перезапуска.
  useEffect(() => {
    if (sessionId) void takeOverSession(sessionId)
  }, [sessionId])

  useEffect(() => {
    markAuxWindow()
    void window.api.settings.get().then((s) => {
      applyUiTheme(s.theme)
      document.documentElement.dataset.density = s.density ?? 'comfortable'
      if (s.sftpWidth) setSftpWidth(s.sftpWidth)
    })
    void window.api.servers.list().then(setServers)
    // Закрываем соединение только когда уходит само окно. При возврате вкладки его
    // забирает главное окно, при переподключении старую закрывает `doReconnect`.
    return () => {
      const sid = sessionRef.current
      if (!sid || reattachingRef.current) return
      void window.api.session.close(sid)
    }
  }, [])

  // Главное окно слушает эти же события; откреплённое не слушало ничего — отсюда и
  // ощущение сломанного окна после обрыва.
  useEffect(() => {
    const offStatus = window.api.session.onStatus((p) => {
      if (p.id === sessionId) setStatus(p.status)
    })
    const offExit = window.api.session.onExit((p) => {
      if (p.id !== sessionId) return
      setStatus('error')
      // Панели Docker/логов/процессов без живой сессии показывать нечего: возвращаем
      // окно на терминал, где хотя бы видно, что произошло, и есть «Переподключить».
      setTool('terminal')
    })
    return () => {
      offStatus()
      offExit()
    }
  }, [sessionId])

  // Крестик в шапке зовёт `WebviewWindow.close()` — webview сносится целиком, и
  // cleanup React-эффекта отработать не успевает. Поэтому закрытие перехватываем:
  // сначала гасим сессию, потом добиваем окно. Иначе SSH-соединение висело на сервере
  // до выхода из приложения.
  useEffect(() => {
    const w = getCurrentWindow()
    let un: (() => void) | undefined
    void w
      .onCloseRequested(async (e) => {
        const sid = sessionRef.current
        // При возврате вкладки сессию забирает главное окно — трогать её нельзя.
        if (reattachingRef.current || !sid) return
        e.preventDefault()
        try {
          await window.api.session.close(sid)
        } catch {
          /* окно всё равно закрываем */
        }
        await w.destroy()
      })
      .then((u) => {
        un = u
      })
    return () => un?.()
  }, [])

  /** Поднять сессию заново на том же сервере. Кнопка была, а обработчика у неё не было. */
  const doReconnect = useCallback(async () => {
    if (!serverId) return
    const dead = sessionId
    setStatus('connecting')
    try {
      const id = await window.api.session.openSsh({ serverId, cols: 80, rows: 24 })
      setSessionId(id)
      setStatus('connected')
      // Прежнюю закрываем только после успеха: иначе неудачная попытка оставила бы
      // окно вообще без сессии, и вернуть вкладку было бы уже нечем.
      if (dead) {
        void window.api.session.close(dead)
      }
    } catch {
      setStatus('error')
    }
  }, [serverId, sessionId])

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

  // Раскладка теперь такая же, как в главном окне: строка из flex-элементов, ширины
  // задают сами классы (`.pane-area` тянется, `.sftp-resizer` 5px, `.sftp-panel`
  // фиксированной ширины). Прежняя сетка с явными номерами колонок требовала, чтобы
  // разметка внутри знала о своём месте снаружи, — а вся суть общего компонента в том,
  // чтобы она об этом не знала.
  const bodyStyle: CSSProperties = {
    flex: 1,
    minHeight: 0,
    minWidth: 0,
    overflow: 'hidden',
    display: 'flex',
    flexDirection: 'row',
    alignItems: 'stretch'
  }

  const slotStyle: CSSProperties = {
    flex: 1,
    minWidth: 0,
    minHeight: 0,
    overflow: 'hidden',
    display: 'flex',
    flexDirection: 'row',
    alignItems: 'stretch'
  }

  const doReattach = (): void => {
    reattachingRef.current = true
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
        <div ref={slotRef} className="detached-tab-slot" style={slotStyle}>
          <ServerWorkspace
            leaf={showRail ? leaf : undefined}
            server={server}
            title={title}
            tool={tool}
            onSelectTool={setTool}
            onReconnect={() => void doReconnect()}
            onEditServer={() => {}}
            sessionId={status === 'connected' ? sessionId || undefined : undefined}
            serverId={serverId || undefined}
            sftpOpen={sftpOpen}
            sftpWidth={sftpWidth}
            onSftpClose={() => setSftpOpen(false)}
            onSftpResizeStart={startSftpResize}
            terminal={
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
            }
          />
        </div>
      </div>
    </div>
  )
}
