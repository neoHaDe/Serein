import { useCallback, useEffect, useRef, useState } from 'react'
import type {
  DockerContainer,
  DockerAction,
  DockerContainerStats,
  DockerContainerFileEntry,
  DockerComposeProject,
  DockerComposeService,
  DockerComposeAction
} from '../../shared/types'
import { Icon } from './Icon'
import { DockerLogView, openDetachedLogsWindow, useLogsPanelResize } from './dockerLogs'
import { useCtrlWheelZoom } from '../useCtrlWheelZoom'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'

interface Props {
  /** SSH-сессия, на которой выполняем docker-команды и shell. */
  sessionId: string
  serverId?: string
  panelTitle?: string
  onClose: () => void
  /** В рельсе workspace — без попапа и backdrop. */
  docked?: boolean
  fill?: boolean
  /** После «shell в контейнер» переключить вкладку на Terminal. */
  onGoTerminal?: () => void
  /** После открепления панели в отдельное окно. */
  onDetached?: () => void
}

function formatPorts(raw?: string): string {
  if (!raw?.trim()) return '—'
  return raw
    .split(',')
    .map((part) => {
      const m = part.trim().match(/:(\d+)->(\d+)/)
      return m ? `${m[1]} → ${m[2]}` : part.trim()
    })
    .join(', ')
}

function formatCreated(raw?: string): string {
  if (!raw?.trim()) return '—'
  const i = raw.indexOf(' +')
  return i > 0 ? raw.slice(0, i) : raw
}

type DockerStateTone = 'run' | 'warn' | 'stop'

function dockerStateTone(state: string, status?: string): DockerStateTone {
  const s = state.toLowerCase().trim()
  const st = (status ?? '').toLowerCase()
  if (s === 'running' || (s === '' && st.startsWith('up'))) return 'run'
  if (s === 'restarting' || s === 'starting' || st.includes('restarting') || st.includes('starting')) return 'warn'
  return 'stop'
}

function dockerStateDot(tone: DockerStateTone): string {
  if (tone === 'run') return 'var(--green)'
  if (tone === 'warn') return 'var(--warn)'
  return 'var(--danger)'
}

function running(state: string, status?: string): boolean {
  return dockerStateTone(state, status) === 'run'
}

type DockerHealth = 'ok' | 'bad' | 'warn'

function dockerHealth(status?: string): DockerHealth | null {
  const s = (status ?? '').toLowerCase()
  if (s.includes('(healthy)')) return 'ok'
  if (s.includes('(unhealthy)')) return 'bad'
  if (s.includes('health: starting')) return 'warn'
  return null
}

function healthLabel(h: DockerHealth): string {
  if (h === 'ok') return 'healthy'
  if (h === 'bad') return 'unhealthy'
  return 'starting'
}

function matchContainerFilter(c: DockerContainer, q: string): boolean {
  const f = q.trim().toLowerCase()
  if (!f) return true
  return (
    c.name.toLowerCase().includes(f) ||
    c.id.toLowerCase().includes(f) ||
    c.state.toLowerCase().includes(f) ||
    c.status.toLowerCase().includes(f) ||
    (c.image ?? '').toLowerCase().includes(f)
  )
}

function joinContainerPath(base: string, name: string): string {
  if (base === '/') return `/${name}`
  return `${base.replace(/\/$/, '')}/${name}`
}

function DockerPropsModal({
  container: c,
  onClose
}: {
  container: DockerContainer
  onClose: () => void
}): JSX.Element {
  const rows = [
    { k: 'Имя', v: c.name },
    { k: 'ID', v: c.id },
    { k: 'Образ', v: c.image },
    { k: 'State', v: c.state, tone: dockerStateTone(c.state, c.status) },
    { k: 'Status', v: c.status },
    { k: 'Порты', v: formatPorts(c.ports) },
    { k: 'Создан', v: formatCreated(c.created) }
  ]
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal sftp-props-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Свойства контейнера</h2>
        <dl className="sftp-props">
          {rows.map((r) => (
            <div key={r.k} className="sftp-props-row">
              <dt>{r.k}</dt>
              <dd title={r.v} className={r.tone ? `docker-state is-${r.tone}` : undefined}>{r.v || '—'}</dd>
            </div>
          ))}
        </dl>
        <div className="modal-actions">
          <button className="primary" onClick={onClose}>
            Закрыть
          </button>
        </div>
      </div>
    </div>
  )
}

function DockerContainerFiles({
  containerId,
  path,
  entries,
  loading,
  error,
  onNavigate,
  onReload
}: {
  containerId: string
  path: string
  entries: DockerContainerFileEntry[]
  loading: boolean
  error: string | null
  onNavigate: (path: string) => void
  onReload: () => void
}): JSX.Element {
  const crumbs = path === '/' ? [{ label: '/', path: '/' }] : [{ label: '/', path: '/' }, ...path.split('/').filter(Boolean).map((p, i, arr) => ({
    label: p,
    path: `/${arr.slice(0, i + 1).join('/')}`
  }))]
  return (
    <div className="docker-files">
      <div className="docker-files-head">
        <div className="docker-files-crumbs">
          {crumbs.map((c, i) => (
            <span key={c.path}>
              {i > 0 && <span className="docker-files-sep">/</span>}
              <button type="button" className="docker-crumb" onClick={() => onNavigate(c.path)}>{c.label}</button>
            </span>
          ))}
        </div>
        <button type="button" className="mini" title="Обновить" onClick={onReload}>
          <Icon name="refresh" size={14} />
        </button>
      </div>
      {loading && <div className="hint" style={{ padding: 8 }}>Загрузка…</div>}
      {error && <div className="sftp-error" style={{ margin: 8 }}>{error}</div>}
      {!loading && !error && entries.length === 0 && (
        <div className="hint" style={{ padding: 8 }}>Пусто.</div>
      )}
      <div className="docker-files-list">
        {path !== '/' && (
          <button type="button" className="docker-files-row" onClick={() => onNavigate(path.replace(/\/[^/]+$/, '') || '/')}>
            <Icon name="back" size={14} /> ..
          </button>
        )}
        {entries.map((e) => (
          <button
            key={e.name}
            type="button"
            className="docker-files-row"
            onClick={() => {
              if (e.kind === 'dir') onNavigate(joinContainerPath(path, e.name))
            }}
            disabled={e.kind !== 'dir'}
            title={e.kind === 'dir' ? 'Открыть' : e.name}
          >
            <Icon name={e.kind === 'dir' ? 'folder' : e.kind === 'link' ? 'link' : 'file'} size={14} />
            <span>{e.name}{e.kind === 'dir' ? '/' : ''}</span>
          </button>
        ))}
      </div>
      <div className="hint docker-files-id">Контейнер {containerId}</div>
    </div>
  )
}

function DockerDetail({
  container: c,
  busy,
  stats,
  statsError,
  logsText,
  following,
  detailPane,
  filesPath,
  filesEntries,
  filesLoading,
  filesError,
  onPaneChange,
  onFilesNavigate,
  onFilesReload,
  onAction,
  onShell,
  onStopFollow,
  onDetachLogs,
  onRefreshLogs
}: {
  container: DockerContainer
  busy: string | null
  stats: DockerContainerStats | null
  statsError: string | null
  logsText: string
  following: boolean
  detailPane: 'logs' | 'files'
  filesPath: string
  filesEntries: DockerContainerFileEntry[]
  filesLoading: boolean
  filesError: string | null
  onPaneChange: (pane: 'logs' | 'files') => void
  onFilesNavigate: (path: string) => void
  onFilesReload: () => void
  onAction: (action: DockerAction) => void
  onShell: () => void
  onStopFollow: () => void
  onDetachLogs: () => void
  onRefreshLogs: () => void
}): JSX.Element {
  const { zoom, ref: logsZoomRef, reset: resetLogsZoom } = useCtrlWheelZoom('serein.logs.zoom')
  const tone = dockerStateTone(c.state, c.status)
  const isRunning = running(c.state, c.status)
  const health = dockerHealth(c.status)
  const rows: { k: string; v: string; tone?: DockerStateTone }[] = [
    { k: 'Status', v: c.status || c.state },
    { k: 'State', v: c.state, tone },
    { k: 'CPU', v: isRunning ? stats?.cpuPct || '…' : '—' },
    { k: 'Memory', v: isRunning ? stats?.memUsage || '…' : '—' },
    { k: 'Ports', v: formatPorts(c.ports) },
    { k: 'Image', v: c.image || '—' },
    { k: 'Created', v: formatCreated(c.created) }
  ]
  return (
    <div className="docker-detail">
      <div className="docker-detail-top">
        {statsError && isRunning && (
          <div className="sftp-error" style={{ margin: '0 0 8px' }}>
            {statsError}
          </div>
        )}
        <dl className="docker-detail-grid">
          {rows.map((r) => (
            <div key={r.k} className="docker-detail-row">
              <dt>{r.k}</dt>
              <dd title={r.v} className={r.tone ? `docker-state is-${r.tone}` : undefined}>{r.v}</dd>
            </div>
          ))}
        </dl>
        <div className="docker-detail-actions">
          {isRunning && (
            <button className="mini" onClick={onShell}>
              <Icon name="shell" size={14} /> Shell
            </button>
          )}
          <button className={`mini${detailPane === 'files' ? ' active' : ''}`} onClick={() => onPaneChange('files')}>
            <Icon name="folder" size={14} /> Files
          </button>
          {isRunning ? (
            <>
              <button className="mini" disabled={busy === c.id} onClick={() => onAction('restart')}>
                <Icon name="restart" size={14} /> Restart
              </button>
              <button className="mini" disabled={busy === c.id} onClick={() => onAction('stop')}>
                <Icon name="stop" size={14} /> Stop
              </button>
            </>
          ) : (
            <>
              <button className="mini" disabled={busy === c.id} onClick={() => onAction('start')}>
                <Icon name="play" size={14} /> Start
              </button>
              <button className="mini danger" disabled={busy === c.id} onClick={() => onAction('remove')}>
                <Icon name="trash" size={14} /> Remove
              </button>
            </>
          )}
        </div>
        {health && (
          <div className={`docker-health is-${health}`}>{healthLabel(health)}</div>
        )}
        <div className="hint docker-detail-id">ID {c.id}</div>
      </div>
      <div className="docker-detail-bottom">
        <div className="docker-detail-tabs">
          <button type="button" className={detailPane === 'logs' ? 'active' : ''} onClick={() => onPaneChange('logs')}>
            <Icon name="logs" size={14} /> Логи
          </button>
          <button type="button" className={detailPane === 'files' ? 'active' : ''} onClick={() => onPaneChange('files')}>
            <Icon name="folder" size={14} /> Files
          </button>
        </div>
        {detailPane === 'logs' ? (
      <div className="docker-detail-logs">
        <div className="docker-detail-logs-head">
          <span><Icon name="logs" size={14} /> Логи</span>
          <div className="docker-detail-logs-actions">
            {following && (
              <button type="button" className="mini" onClick={onStopFollow}>
                Стоп
              </button>
            )}
            <button type="button" className="mini" title="Масштаб (Ctrl+колесо)" onClick={resetLogsZoom}>
              {Math.round(zoom * 100)}%
            </button>
            <button type="button" className="mini" title="Обновить" onClick={onRefreshLogs}>
              <Icon name="refresh" size={14} />
            </button>
            <button type="button" className="mini" title="Открепить в отдельное окно" onClick={onDetachLogs}>
              <Icon name="external" size={14} />
            </button>
          </div>
        </div>
        <div className="docker-detail-logs-scroll">
          <DockerLogView
            text={logsText}
            follow={following}
            fill
            zoom={zoom}
            zoomRef={logsZoomRef}
          />
        </div>
      </div>
        ) : (
          <DockerContainerFiles
            containerId={c.id}
            path={filesPath}
            entries={filesEntries}
            loading={filesLoading}
            error={filesError}
            onNavigate={onFilesNavigate}
            onReload={onFilesReload}
          />
        )}
      </div>
    </div>
  )
}

function projectName(p: DockerComposeProject): string {
  return p.project || p.name
}

function DockerComposeView({
  sessionId,
  onGoTerminal,
  onError
}: {
  sessionId: string
  onGoTerminal?: () => void
  onError: (msg: string | null) => void
}): JSX.Element {
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [projects, setProjects] = useState<DockerComposeProject[]>([])
  const [selected, setSelected] = useState<DockerComposeProject | null>(null)
  const [services, setServices] = useState<DockerComposeService[]>([])
  const [svcLoading, setSvcLoading] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const [yamlFor, setYamlFor] = useState<string | null>(null)
  const [yamlText, setYamlText] = useState('')
  const [logsFor, setLogsFor] = useState<{ composeFile: string; service: string; label: string } | null>(null)
  const [logsText, setLogsText] = useState('')
  const [following, setFollowing] = useState(false)
  const logsKeyRef = useRef<string | null>(null)
  const LOADING = 'Загрузка логов…'

  const reloadProjects = useCallback(async () => {
    setLoading(true)
    try {
      const res = await window.api.docker.composeList(sessionId)
      if (res.ok) {
        setProjects(res.projects ?? [])
        setError(null)
        onError(null)
        setSelected((prev) => {
          if (!prev) return null
          return (res.projects ?? []).find((p) => p.name === prev.name || p.composeFile === prev.composeFile) ?? null
        })
      } else {
        const msg = res.error ?? 'Ошибка compose'
        setError(msg)
        onError(msg)
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      setError(msg)
      onError(msg)
    } finally {
      setLoading(false)
    }
  }, [sessionId, onError])

  const reloadServices = useCallback(async (p: DockerComposeProject) => {
    setSvcLoading(true)
    try {
      const res = await window.api.docker.composePs(sessionId, p.composeFile, projectName(p))
      if (res.ok) {
        setServices(res.services ?? [])
        setError(null)
        onError(null)
      } else {
        const msg = res.error ?? 'Ошибка ps'
        setError(msg)
        onError(msg)
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      setError(msg)
      onError(msg)
    } finally {
      setSvcLoading(false)
    }
  }, [sessionId, onError])

  useEffect(() => {
    void reloadProjects()
  }, [sessionId]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!selected) {
      setServices([])
      return
    }
    void reloadServices(selected)
  }, [selected, reloadServices])

  const doComposeAction = async (
    composeFile: string,
    project: string,
    action: DockerComposeAction,
    service?: string,
    confirmMsg?: string
  ): Promise<void> => {
    if (confirmMsg && !confirm(confirmMsg)) return
    const key = service ? `${composeFile}:${service}:${action}` : `${composeFile}:${action}`
    setBusy(key)
    const res = await window.api.docker.composeAction(sessionId, composeFile, project, action, service)
    setBusy(null)
    if (!res.ok) {
      const msg = res.error ?? 'Ошибка'
      setError(msg)
      onError(msg)
    } else {
      void reloadProjects()
      if (selected?.composeFile === composeFile) void reloadServices(selected)
    }
  }

  const openYaml = async (composeFile: string): Promise<void> => {
    const res = await window.api.docker.composeRead(sessionId, composeFile)
    if (res.ok) {
      setYamlText(res.text ?? '')
      setYamlFor(composeFile)
    } else {
      const msg = res.error ?? 'Не удалось прочитать файл'
      setError(msg)
      onError(msg)
    }
  }

  const beginComposeLogs = useCallback(async (composeFile: string, project: string, service: string, label: string) => {
    if (logsKeyRef.current) void window.api.docker.cancelComposeLogs(sessionId)
    const key = `${composeFile}:${service}`
    logsKeyRef.current = key
    setLogsFor({ composeFile, service, label })
    setFollowing(true)
    setLogsText(LOADING)
    const res = await window.api.docker.composeLogs(sessionId, composeFile, project, service)
    if (logsKeyRef.current !== key) return
    setFollowing(false)
    if (res.ok) setLogsText((t) => (t === LOADING ? res.logs || '(пусто)' : t))
    else setLogsText((t) => (t === LOADING ? `Ошибка: ${res.error}` : t))
  }, [sessionId])

  useEffect(() => {
    return window.api.docker.onLogs((p) => {
      if (p.sessionId !== sessionId || !logsKeyRef.current) return
      if (!p.containerId.startsWith('compose:')) return
      setLogsText((t) => {
        const base = t === LOADING ? '' : t
        const next = base + p.chunk
        return next.length > 400000 ? next.slice(-350000) : next
      })
    })
  }, [sessionId])

  const stopComposeLogs = (): void => {
    if (logsKeyRef.current) void window.api.docker.cancelComposeLogs(sessionId)
    logsKeyRef.current = null
    setFollowing(false)
    setLogsFor(null)
    setLogsText('')
  }

  const openComposeShell = (svc: DockerComposeService): void => {
    if (!svc.id) return
    window.api.session.write(sessionId, `docker exec -it ${svc.id} sh\n`)
    onGoTerminal?.()
  }

  if (logsFor) {
    return (
      <div className="docker-compose-logs fill">
        <div className="docker-detail-logs-head">
          <span><Icon name="logs" size={14} /> {logsFor.label}</span>
          <div className="docker-detail-logs-actions">
            {following && (
              <button type="button" className="mini" onClick={() => {
                void window.api.docker.cancelComposeLogs(sessionId, logsFor.composeFile, logsFor.service)
                setFollowing(false)
              }}>Стоп</button>
            )}
            <button type="button" className="mini" onClick={stopComposeLogs}><Icon name="back" size={14} /> назад</button>
          </div>
        </div>
        <DockerLogView text={logsText} follow={following} fill />
      </div>
    )
  }

  if (selected) {
    return (
      <div className="docker-compose-detail fill">
        <div className="docker-compose-head">
          <button type="button" className="mini" onClick={() => setSelected(null)}><Icon name="back" size={14} /> проекты</button>
          <span className="docker-compose-title" title={selected.composeFile}>{selected.name}</span>
          <button type="button" className="mini" title="compose-файл" onClick={() => void openYaml(selected.composeFile)}>
            <Icon name="file" size={14} /> YAML
          </button>
          <button type="button" className="mini" disabled={busy === `${selected.composeFile}:up`} onClick={() => void doComposeAction(selected.composeFile, projectName(selected), 'up')}>
            <Icon name="play" size={14} /> Up
          </button>
          <button type="button" className="mini" disabled={busy === `${selected.composeFile}:down`} onClick={() => void doComposeAction(selected.composeFile, projectName(selected), 'down', undefined, `Остановить стек «${selected.name}»?`)}>
            <Icon name="stop" size={14} /> Down
          </button>
          <button type="button" className="mini" onClick={() => void reloadServices(selected)}><Icon name="refresh" size={14} /></button>
        </div>
        {error && <div className="sftp-error" onClick={() => setError(null)}>{error}</div>}
        {svcLoading && <div className="hint" style={{ padding: 8 }}>Загрузка сервисов…</div>}
        <div className="sftp-list docker-table docker-compose-table">
          <div className="sftp-row sftp-row-head docker-row-head">
            <span className="docker-th">Service</span>
            <span className="docker-th">State</span>
            <span className="docker-th">Status</span>
            <span className="docker-th">Ports</span>
            <span className="docker-th docker-th-actions"> </span>
          </div>
          {services.length === 0 && !svcLoading ? (
            <div className="ws-empty">Сервисов нет.</div>
          ) : (
            services.map((svc) => {
              const tone = dockerStateTone(svc.state, svc.status)
              const isRunning = running(svc.state, svc.status)
              const actKey = `${selected.composeFile}:${svc.service}`
              return (
                <div key={`${svc.service}-${svc.id}`} className="sftp-row docker-data-row">
                  <span className="docker-namecell">
                    <span className="docker-dot" style={{ background: dockerStateDot(tone) }} />
                    <span className="docker-name">{svc.service || svc.name}</span>
                  </span>
                  <span className={`docker-state is-${tone}`}>{svc.state}</span>
                  <span className="docker-status" title={svc.status}>{svc.status}</span>
                  <span title={svc.ports}>{svc.ports ? formatPorts(svc.ports) : '—'}</span>
                  <span className="docker-compose-svc-actions">
                    {isRunning ? (
                      <>
                        <button className="mini" title="Shell" onClick={() => openComposeShell(svc)}><Icon name="shell" size={14} /></button>
                        <button className="mini" title="Логи" onClick={() => void beginComposeLogs(selected.composeFile, projectName(selected), svc.service, `${selected.name}/${svc.service}`)}><Icon name="logs" size={14} /></button>
                        <button className="mini" disabled={busy === `${actKey}:restart`} onClick={() => void doComposeAction(selected.composeFile, projectName(selected), 'restart', svc.service)}><Icon name="restart" size={14} /></button>
                        <button className="mini" disabled={busy === `${actKey}:stop`} onClick={() => void doComposeAction(selected.composeFile, projectName(selected), 'stop', svc.service)}><Icon name="stop" size={14} /></button>
                      </>
                    ) : (
                      <>
                        <button className="mini" title="Логи" onClick={() => void beginComposeLogs(selected.composeFile, projectName(selected), svc.service, `${selected.name}/${svc.service}`)}><Icon name="logs" size={14} /></button>
                        <button className="mini" disabled={busy === `${actKey}:start`} onClick={() => void doComposeAction(selected.composeFile, projectName(selected), 'start', svc.service)}><Icon name="play" size={14} /></button>
                      </>
                    )}
                  </span>
                </div>
              )
            })
          )}
        </div>
        {yamlFor && (
          <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && setYamlFor(null)}>
            <div className="modal docker-compose-yaml" onClick={(e) => e.stopPropagation()}>
              <h2>Compose</h2>
              <pre className="docker-compose-yaml-text">{yamlText}</pre>
              <div className="modal-actions">
                <button className="primary" onClick={() => setYamlFor(null)}>Закрыть</button>
              </div>
            </div>
          </div>
        )}
      </div>
    )
  }

  return (
    <div className="docker-compose-list fill">
      {loading && <div className="hint" style={{ padding: '10px 12px' }}>Загрузка…</div>}
      {error && <div className="sftp-error" onClick={() => setError(null)}>{error}</div>}
      {!loading && !error && projects.length === 0 && (
        <div className="ws-empty">Compose-проектов нет.</div>
      )}
      {projects.map((p) => (
        <button
          key={p.composeFile || p.name}
          type="button"
          className="docker-compose-project"
          onClick={() => setSelected(p)}
          title={p.composeFile}
        >
          <span className="docker-compose-project-name">{p.name}</span>
          <span className="docker-compose-project-status">{p.status || '—'}</span>
          <span className="docker-compose-project-file">{p.composeFile ? p.composeFile.replace(/^.*\//, '…/') : '—'}</span>
        </button>
      ))}
    </div>
  )
}

export function DockerPanel({
  sessionId,
  serverId,
  panelTitle,
  onClose,
  docked,
  fill,
  onGoTerminal,
  onDetached
}: Props): JSX.Element {
  const [containers, setContainers] = useState<DockerContainer[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)
  const [selected, setSelected] = useState<DockerContainer | null>(null)
  const [stats, setStats] = useState<DockerContainerStats | null>(null)
  const [statsError, setStatsError] = useState<string | null>(null)
  const [logsFor, setLogsFor] = useState<DockerContainer | null>(null)
  const [logsText, setLogsText] = useState('')
  const [following, setFollowing] = useState(false)
  const [ctx, setCtx] = useState<{ x: number; y: number; container: DockerContainer } | null>(null)
  const [propsFor, setPropsFor] = useState<DockerContainer | null>(null)
  const [mainTab, setMainTab] = useState<'containers' | 'compose'>('containers')
  const [filter, setFilter] = useState('')
  const [detailPane, setDetailPane] = useState<'logs' | 'files'>('logs')
  const [filesPath, setFilesPath] = useState('/')
  const [filesEntries, setFilesEntries] = useState<DockerContainerFileEntry[]>([])
  const [filesLoading, setFilesLoading] = useState(false)
  const [filesError, setFilesError] = useState<string | null>(null)
  const logsForRef = useRef<string | null>(null)
  const skipCancelRef = useRef(false)
  const { size, onResizeDown } = useLogsPanelResize()
  const { zoom, ref: zoomRef, reset } = useCtrlWheelZoom('serein.logs.zoom')
  const LOADING = 'Загрузка логов…'

  const reload = useCallback(async () => {
    setLoading(true)
    const res = await window.api.docker.list(sessionId)
    setLoading(false)
    if (res.ok) {
      const list = res.containers ?? []
      setContainers(list)
      setError(null)
      if (selected) {
        const next = list.find((x) => x.id === selected.id)
        if (next) setSelected(next)
        else setSelected(null)
      }
    } else setError(res.error ?? 'Ошибка')
  }, [sessionId, selected])

  const reloadStats = useCallback(async (c: DockerContainer) => {
    if (!running(c.state, c.status)) {
      setStats(null)
      setStatsError(null)
      return
    }
    const res = await window.api.docker.stats(sessionId, c.id)
    if (res.ok && res.stats) {
      setStats(res.stats)
      setStatsError(null)
    } else {
      setStats(null)
      setStatsError(res.error ?? 'Не удалось получить stats')
    }
  }, [sessionId])

  useEffect(() => {
    void reload()
  }, [sessionId]) // eslint-disable-line react-hooks/exhaustive-deps -- reload on session change only

  useEffect(() => {
    if (!docked || !selected) {
      setStats(null)
      setStatsError(null)
      return
    }
    void reloadStats(selected)
    const t = window.setInterval(() => void reloadStats(selected), 3000)
    return () => window.clearInterval(t)
  }, [docked, selected, reloadStats])

  const beginLogs = useCallback(
    async (c: DockerContainer, opts?: { fullscreen?: boolean }) => {
      if (logsForRef.current) void window.api.docker.cancelLogs(sessionId, logsForRef.current)
      logsForRef.current = c.id
      if (opts?.fullscreen) setLogsFor(c)
      setFollowing(true)
      setLogsText(LOADING)
      const res = await window.api.docker.logs(sessionId, c.id)
      if (logsForRef.current !== c.id) return
      setFollowing(false)
      if (res.ok) {
        setLogsText((t) => (t === LOADING ? res.logs || '(пусто)' : t))
      } else {
        setLogsText((t) => (t === LOADING ? `Ошибка: ${res.error}` : t))
      }
    },
    [sessionId]
  )

  const reloadFiles = useCallback(async (c: DockerContainer, path: string) => {
    setFilesLoading(true)
    const res = await window.api.docker.files(sessionId, c.id, path)
    setFilesLoading(false)
    if (res.ok) {
      setFilesPath(res.path ?? path)
      setFilesEntries(res.entries ?? [])
      setFilesError(null)
    } else {
      setFilesEntries([])
      setFilesError(res.error ?? 'Ошибка')
    }
  }, [sessionId])

  useEffect(() => {
    if (!selected) {
      setDetailPane('logs')
      setFilesPath('/')
      setFilesEntries([])
      setFilesError(null)
    }
  }, [selected?.id])

  useEffect(() => {
    if (!docked || !selected || detailPane !== 'files') return
    void reloadFiles(selected, filesPath)
  }, [docked, selected?.id, detailPane, filesPath, reloadFiles])

  useEffect(() => {
    if (!docked || !selected || detailPane !== 'logs') {
      if (logsForRef.current && detailPane !== 'logs') {
        void window.api.docker.cancelLogs(sessionId, logsForRef.current)
        logsForRef.current = null
        setFollowing(false)
      }
      return
    }
    void beginLogs(selected)
    return () => {
      if (logsForRef.current) {
        void window.api.docker.cancelLogs(sessionId, logsForRef.current)
        logsForRef.current = null
      }
      setFollowing(false)
      setLogsText('')
    }
  }, [docked, selected?.id, detailPane, sessionId, beginLogs])

  useEffect(() => {
    return window.api.docker.onLogs((p) => {
      if (p.sessionId !== sessionId) return
      const want = logsForRef.current
      if (!want) return
      if (p.containerId !== want && !p.containerId.startsWith(want)) return
      setLogsText((t) => {
        const base = t === LOADING ? '' : t
        const next = base + p.chunk
        return next.length > 400000 ? next.slice(-350000) : next
      })
    })
  }, [sessionId])

  useEffect(() => {
    return () => {
      if (!skipCancelRef.current) void window.api.docker.cancelLogs(sessionId)
    }
  }, [sessionId])

  const doAction = async (c: DockerContainer, action: DockerAction): Promise<void> => {
    if (action === 'remove' && !confirm(`Удалить контейнер «${c.name}»?`)) return
    setBusy(c.id)
    const res = await window.api.docker.action(sessionId, c.id, action)
    setBusy(null)
    if (!res.ok) setError(res.error ?? 'Ошибка')
    else {
      if (action === 'remove' && selected?.id === c.id) setSelected(null)
      void reload()
    }
  }

  const openLogs = async (c: DockerContainer): Promise<void> => {
    await beginLogs(c, { fullscreen: true })
  }

  const stopLogs = (): void => {
    if (logsForRef.current) void window.api.docker.cancelLogs(sessionId, logsForRef.current)
    logsForRef.current = null
    setFollowing(false)
    setLogsFor(null)
    setLogsText('')
  }

  const stopFollow = (): void => {
    if (logsForRef.current) void window.api.docker.cancelLogs(sessionId, logsForRef.current)
    setFollowing(false)
  }

  const detachLogs = async (c: DockerContainer): Promise<void> => {
    skipCancelRef.current = true
    if (logsForRef.current) void window.api.docker.cancelLogs(sessionId, logsForRef.current)
    logsForRef.current = null
    try {
      await openDetachedLogsWindow({
        sessionId,
        serverId,
        containerId: c.id,
        name: c.name,
        width: size.w,
        height: size.h
      })
    } catch (e) {
      skipCancelRef.current = false
      setFollowing(false)
      setLogsText(`Не удалось открепить: ${e instanceof Error ? e.message : String(e)}`)
      if (docked && selected?.id === c.id) void beginLogs(c)
      return
    }
    setFollowing(false)
    setLogsFor(null)
    if (docked && selected?.id === c.id) {
      skipCancelRef.current = false
      void beginLogs(c)
    } else if (!docked) onClose()
  }

  const closeDetail = (): void => {
    if (logsForRef.current) void window.api.docker.cancelLogs(sessionId, logsForRef.current)
    logsForRef.current = null
    setFollowing(false)
    setLogsText('')
    setSelected(null)
  }

  const openShell = (c: DockerContainer): void => {
    window.api.session.write(sessionId, `docker exec -it ${c.id} sh\n`)
    onGoTerminal?.()
    if (!docked) onClose()
  }

  const detachPanel = async (): Promise<void> => {
    if (!panelTitle) return
    await openDetachedWorkspace({ tool: 'docker', sessionId, serverId, title: panelTitle })
    onDetached?.()
  }

  const openContainer = (c: DockerContainer): void => {
    if (docked) {
      setSelected(c)
      setDetailPane('logs')
      setFilesPath('/')
    }
  }

  const filteredContainers = containers.filter((c) => matchContainerFilter(c, filter))

  const headTitle = logsFor
    ? `Логи: ${logsFor.name}`
    : selected && docked && mainTab === 'containers'
      ? selected.name
      : mainTab === 'compose'
        ? 'Compose'
        : 'Docker'

  return (
    <>
      {!docked && <div className="split-menu-backdrop" onClick={onClose} />}
      <div
        className={
          docked
            ? `ws-panel${fill ? ' fill' : ''}${logsFor ? ' is-logs' : ''}${selected && !logsFor ? ' is-detail' : ''}`
            : `docker-panel${logsFor ? ' is-logs' : ''}`
        }
        ref={logsFor ? zoomRef : undefined}
        style={logsFor && !docked ? { width: size.w, height: size.h, zoom } : undefined}
      >
        <div className={docked ? 'ws-head' : 'tunnel-menu-header'}>
          <span className={docked ? 'ws-head-title' : 'tunnel-menu-title'} title={logsFor ? logsFor.name : selected?.name}>
            {docked && <Icon name={logsFor ? 'logs' : 'docker'} size={15} />}
            {headTitle}
          </span>
          {logsFor ? (
            <>
              {following && (
                <button
                  className="mini"
                  onClick={() => {
                    if (logsForRef.current) void window.api.docker.cancelLogs(sessionId, logsForRef.current)
                    setFollowing(false)
                  }}
                >
                  Стоп
                </button>
              )}
              <button className="mini" title="Масштаб" onClick={reset}>{Math.round(zoom * 100)}%</button>
              <button className="mini" title="Открепить в отдельное окно" onClick={() => logsFor && void detachLogs(logsFor)}>
                <Icon name="external" size={14} />
              </button>
              <button className="mini" onClick={stopLogs}><Icon name="back" size={14} /> назад</button>
            </>
          ) : selected && docked && mainTab === 'containers' ? (
            <>
              <button className="mini" title="Обновить" onClick={() => void reload()}><Icon name="refresh" size={14} /></button>
              <button className="mini" onClick={closeDetail}><Icon name="back" size={14} /> список</button>
            </>
          ) : (
            <>
              {docked && panelTitle && onDetached && !fill && <WsDetachButton onClick={detachPanel} />}
              <button className="mini" title="Обновить" onClick={() => mainTab === 'compose' ? undefined : void reload()} disabled={mainTab === 'compose'}><Icon name="refresh" size={14} /></button>
            </>
          )}
        </div>

        {docked && !logsFor && !selected && (
          <div className="docker-main-tabs">
            <button type="button" className={mainTab === 'containers' ? 'active' : ''} onClick={() => setMainTab('containers')}>
              Containers
            </button>
            <button type="button" className={mainTab === 'compose' ? 'active' : ''} onClick={() => setMainTab('compose')}>
              Compose
            </button>
          </div>
        )}

        {logsFor ? (
          <DockerLogView text={logsText} follow={following} />
        ) : selected && docked && mainTab === 'containers' ? (
          <DockerDetail
            container={selected}
            busy={busy}
            stats={stats}
            statsError={statsError}
            logsText={logsText}
            following={following}
            detailPane={detailPane}
            filesPath={filesPath}
            filesEntries={filesEntries}
            filesLoading={filesLoading}
            filesError={filesError}
            onPaneChange={(pane) => {
              setDetailPane(pane)
              if (pane === 'files' && selected) void reloadFiles(selected, filesPath)
            }}
            onFilesNavigate={(path) => setFilesPath(path)}
            onFilesReload={() => selected && void reloadFiles(selected, filesPath)}
            onAction={(action) => void doAction(selected, action)}
            onShell={() => openShell(selected)}
            onStopFollow={stopFollow}
            onDetachLogs={() => void detachLogs(selected)}
            onRefreshLogs={() => void beginLogs(selected)}
          />
        ) : docked && mainTab === 'compose' ? (
          <div className="docker-compose-wrap">
            <DockerComposeView sessionId={sessionId} onGoTerminal={onGoTerminal} onError={setError} />
          </div>
        ) : docked ? (
          <div className="sftp-list docker-table">
            {loading && <div className="hint" style={{ padding: '10px 12px' }}>Загрузка…</div>}
            {error && <div className="sftp-error" onClick={() => setError(null)}>{error}</div>}
            {!loading && !error && (
              <>
                <div className="docker-filter-row">
                  <input
                    className="docker-filter"
                    placeholder="Поиск…"
                    value={filter}
                    onChange={(e) => setFilter(e.target.value)}
                  />
                </div>
                <div className="sftp-row sftp-row-head docker-row-head">
                  <span className="docker-th">Имя</span>
                  <span className="docker-th">ID</span>
                  <span className="docker-th">State</span>
                  <span className="docker-th">Status</span>
                  <span className="docker-th">Ports</span>
                </div>
                {filteredContainers.length === 0 ? (
                  <div className="ws-empty">{containers.length === 0 ? 'Контейнеров нет.' : 'Ничего не найдено.'}</div>
                ) : (
                  filteredContainers.map((c) => {
                    const health = dockerHealth(c.status)
                    return (
                    <div
                      key={c.id}
                      className="sftp-row docker-data-row is-clickable"
                      title={c.status}
                      onClick={() => openContainer(c)}
                      onContextMenu={(e) => {
                        e.preventDefault()
                        setCtx({ x: e.clientX, y: e.clientY, container: c })
                      }}
                    >
                      <span className="docker-namecell">
                        <span
                          className="docker-dot"
                          style={{ background: dockerStateDot(dockerStateTone(c.state, c.status)) }}
                        />
                        <span className="docker-name">{c.name}</span>
                      </span>
                      <span className="mono" title={c.id}>{c.id}</span>
                      <span className={`docker-state is-${dockerStateTone(c.state, c.status)}`}>{c.state}</span>
                      <span className="docker-status" title={c.status}>
                        {c.status}
                        {health && <span className={`docker-health-badge is-${health}`}>{healthLabel(health)}</span>}
                      </span>
                      <span title={c.ports}>{c.ports ? formatPorts(c.ports) : '—'}</span>
                    </div>
                    )
                  })
                )}
              </>
            )}
          </div>
        ) : (
          <div className="docker-list">
            {loading && <div className="hint" style={{ padding: '10px 12px' }}>Загрузка…</div>}
            {error && <div className="sftp-error" onClick={() => setError(null)}>{error}</div>}
            {!loading && !error && containers.length === 0 && (
              <div className="hint" style={{ padding: '10px 12px' }}>
                Контейнеров нет.
              </div>
            )}
            {containers.map((c) => (
              <div
                key={c.id}
                className="docker-row"
                title={c.status}
                onContextMenu={(e) => {
                  e.preventDefault()
                  setCtx({ x: e.clientX, y: e.clientY, container: c })
                }}
              >
                <span
                  className="docker-dot"
                  style={{ background: dockerStateDot(dockerStateTone(c.state, c.status)) }}
                />
                <div className="docker-info">
                  <div className="docker-name">{c.name}</div>
                </div>
                <div className="docker-actions">
                    {running(c.state, c.status) ? (
                      <>
                        <button className="mini" title="Shell в контейнер" onClick={() => openShell(c)}><Icon name="shell" size={14} /></button>
                        <button className="mini" title="Логи" onClick={() => void openLogs(c)}><Icon name="logs" size={14} /></button>
                        <button className="mini" title="Перезапустить" disabled={busy === c.id} onClick={() => void doAction(c, 'restart')}><Icon name="restart" size={14} /></button>
                        <button className="mini" title="Остановить" disabled={busy === c.id} onClick={() => void doAction(c, 'stop')}><Icon name="stop" size={14} /></button>
                      </>
                    ) : (
                      <>
                        <button className="mini" title="Логи" onClick={() => void openLogs(c)}><Icon name="logs" size={14} /></button>
                        <button className="mini" title="Запустить" disabled={busy === c.id} onClick={() => void doAction(c, 'start')}><Icon name="play" size={14} /></button>
                        <button className="mini danger" title="Удалить" disabled={busy === c.id} onClick={() => void doAction(c, 'remove')}><Icon name="trash" size={14} /></button>
                      </>
                    )}
                </div>
              </div>
            ))}
          </div>
        )}
        {logsFor && !docked && (
          <>
            <div className="docker-resize e" onMouseDown={onResizeDown('e')} />
            <div className="docker-resize s" onMouseDown={onResizeDown('s')} />
            <div className="docker-resize se" onMouseDown={onResizeDown('se')} />
          </>
        )}
      </div>
      {ctx && (
        <>
          <div className="split-menu-backdrop" onClick={() => setCtx(null)} />
          <div
            className="sftp-ctx-menu"
            style={{ left: ctx.x, top: ctx.y }}
            onMouseDown={(e) => e.stopPropagation()}
          >
            {docked && (
              <button
                type="button"
                className="sftp-ctx-item"
                onClick={() => {
                  setSelected(ctx.container)
                  setCtx(null)
                }}
              >
                Открыть
              </button>
            )}
            <button
              type="button"
              className="sftp-ctx-item"
              onClick={() => {
                setPropsFor(ctx.container)
                setCtx(null)
              }}
            >
              Свойства
            </button>
          </div>
        </>
      )}
      {propsFor && <DockerPropsModal container={propsFor} onClose={() => setPropsFor(null)} />}
    </>
  )
}
