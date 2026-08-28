import { useMemo, useState } from 'react'
import type { ServerConfig } from '../../shared/types'
import { Icon } from './Icon'

/** Цвет точки по агрегированному статусу подключения сервера. */
const STATUS_DOT: Record<string, string> = {
  connected: 'var(--green)',
  connecting: '#e0af68',
  reconnecting: '#e0af68',
  error: 'var(--danger)'
}

interface Props {
  servers: ServerConfig[]
  onConnect: (s: ServerConfig) => void
  onOpenLocal: () => void
  onNew: () => void
  onEdit: (s: ServerConfig) => void
  onDelete: (id: string) => void
  onOpenSettings: () => void
  onOpenKeyGen: () => void
  onImport: (kind: 'ssh' | 'putty') => void
  width: number
  collapsed?: boolean
  onToggleCollapse?: () => void
  /** Агрегированный статус подключения по serverId (для живого индикатора). */
  statuses?: Record<string, 'connected' | 'connecting' | 'reconnecting' | 'error'>
}

export function Sidebar({ servers, onConnect, onOpenLocal, onNew, onEdit, onDelete, onOpenSettings, onOpenKeyGen, onImport, width, collapsed, onToggleCollapse, statuses }: Props): JSX.Element {
  const [filter, setFilter] = useState('')
  const [importMenu, setImportMenu] = useState(false)

  const groups = useMemo(() => {
    const q = filter.trim().toLowerCase()
    const filtered = servers.filter(
      (s) =>
        !q ||
        s.name.toLowerCase().includes(q) ||
        s.host.toLowerCase().includes(q) ||
        s.username.toLowerCase().includes(q)
    )
    const map = new Map<string, ServerConfig[]>()
    for (const s of filtered) {
      const g = s.group?.trim() || 'Без группы'
      if (!map.has(g)) map.set(g, [])
      map.get(g)!.push(s)
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]))
  }, [servers, filter])

  return (
    <aside className={'sidebar' + (collapsed ? ' collapsed' : '')} style={{ width: collapsed ? 56 : width }}>
      <div className="sidebar-header">
        {collapsed ? (
          <button className="icon-btn" title="Развернуть список серверов" onClick={onToggleCollapse}>
            <Icon name="chevron-right" />
          </button>
        ) : (
          <>
        <span className="logo">
          <Icon name="logo" size={16} /> Serein
        </span>
        <div className="sidebar-header-actions">
          <button className="icon-btn" title="Свернуть список серверов" onClick={onToggleCollapse}>
            <Icon name="chevron-left" />
          </button>
          <div className="import-control">
            <button className="icon-btn" title="Импортировать серверы" onClick={() => setImportMenu((v) => !v)}>
              <Icon name="import" />
            </button>
            {importMenu && (
              <>
                <div className="split-menu-backdrop" onClick={() => setImportMenu(false)} />
                <div className="import-menu">
                  <button
                    className="split-menu-item"
                    onClick={() => {
                      setImportMenu(false)
                      onImport('ssh')
                    }}
                  >
                    Из ~/.ssh/config
                  </button>
                  <button
                    className="split-menu-item"
                    onClick={() => {
                      setImportMenu(false)
                      onImport('putty')
                    }}
                  >
                    Из сессий PuTTY
                  </button>
                </div>
              </>
            )}
          </div>
          <button className="icon-btn" title="Добавить сервер" onClick={onNew}>
            <Icon name="plus" />
          </button>
        </div>
          </>
        )}
      </div>

      {!collapsed && (
      <input
        className="search"
        placeholder="Поиск серверов…"
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
      />
      )}

      <div className="server-list">
        {!collapsed && groups.length === 0 && <div className="hint">Серверов пока нет. Нажмите «+».</div>}
        {groups.map(([group, items]) => (
          <div key={group} className="group">
            {!collapsed && <div className="group-title">{group}</div>}
            {items.map((s) => (
              <div
                key={s.id}
                className="server-item"
                onClick={collapsed ? () => onConnect(s) : undefined}
                onDoubleClick={() => onConnect(s)}
                title={`${s.name} — ${s.username}@${s.host}:${s.port}`}
              >
                <span className="dot-wrap" title={statuses?.[s.id] ? `Статус: ${statuses[s.id]}` : undefined}>
                  <span className="dot" style={{ background: s.color || '#7aa2f7' }} />
                  {statuses?.[s.id] && (
                    <span
                      className={
                        'dot-status' +
                        (statuses[s.id] === 'connecting' || statuses[s.id] === 'reconnecting' ? ' pulse' : '')
                      }
                      style={{ background: STATUS_DOT[statuses[s.id]] }}
                    />
                  )}
                </span>
                {!collapsed && (
                <>
                <div className="server-info">
                  <div className="server-name">{s.name}</div>
                  <div className="server-host">
                    {s.username}@{s.host}
                  </div>
                </div>
                <div className="server-actions">
                  <button className="mini" title="Подключиться" onClick={() => onConnect(s)}>
                    <Icon name="play" size={14} />
                  </button>
                  <button className="mini" title="Изменить" onClick={() => onEdit(s)}>
                    <Icon name="edit" size={14} />
                  </button>
                  <button
                    className="mini danger"
                    title="Удалить"
                    onClick={() => {
                      if (confirm(`Удалить сервер «${s.name}»?`)) onDelete(s.id)
                    }}
                  >
                    <Icon name="trash" size={14} />
                  </button>
                </div>
                </>
                )}
              </div>
            ))}
          </div>
        ))}
      </div>

      <div className="sidebar-footer">
        {collapsed ? (
          <>
            <button className="icon-btn" title="Локальный терминал" onClick={onOpenLocal}>
              <Icon name="desktop" />
            </button>
            <button className="icon-btn" title="Генерация ключей" onClick={onOpenKeyGen}>
              <Icon name="key" />
            </button>
            <button className="icon-btn" title="Настройки" onClick={onOpenSettings}>
              <Icon name="settings" />
            </button>
          </>
        ) : (
          <>
        <button className="full-btn" onClick={onOpenLocal}>
          <Icon name="desktop" /> Локальный терминал
        </button>
        <button className="full-btn" onClick={onOpenKeyGen}>
          <Icon name="key" /> Генерация ключей
        </button>
        <button className="full-btn" onClick={onOpenSettings}>
          <Icon name="settings" /> Настройки
        </button>
          </>
        )}
      </div>
    </aside>
  )
}
