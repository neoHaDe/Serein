import { useMemo, useState } from 'react'
import type { ServerConfig } from '../../shared/types'
import { Icon } from './Icon'

/** Цвет точки по агрегированному статусу подключения сервера. */
const STATUS_DOT: Record<string, string> = {
  connected: 'var(--green)',
  connecting: '#e0af68',
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
  /** Агрегированный статус подключения по serverId (для живого индикатора). */
  statuses?: Record<string, 'connected' | 'connecting' | 'error'>
}

export function Sidebar({ servers, onConnect, onOpenLocal, onNew, onEdit, onDelete, onOpenSettings, onOpenKeyGen, onImport, width, statuses }: Props): JSX.Element {
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
    <aside className="sidebar" style={{ width }}>
      <div className="sidebar-header">
        <span className="logo">
          <Icon name="logo" size={16} /> TermiNAL
        </span>
        <div className="sidebar-header-actions">
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
      </div>

      <input
        className="search"
        placeholder="Поиск серверов…"
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
      />

      <div className="server-list">
        {groups.length === 0 && <div className="hint">Серверов пока нет. Нажмите «+».</div>}
        {groups.map(([group, items]) => (
          <div key={group} className="group">
            <div className="group-title">{group}</div>
            {items.map((s) => (
              <div
                key={s.id}
                className="server-item"
                onDoubleClick={() => onConnect(s)}
                title={`${s.username}@${s.host}:${s.port}`}
              >
                <span className="dot-wrap" title={statuses?.[s.id] ? `Статус: ${statuses[s.id]}` : undefined}>
                  <span className="dot" style={{ background: s.color || '#7aa2f7' }} />
                  {statuses?.[s.id] && (
                    <span
                      className={'dot-status' + (statuses[s.id] === 'connecting' ? ' pulse' : '')}
                      style={{ background: STATUS_DOT[statuses[s.id]] }}
                    />
                  )}
                </span>
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
              </div>
            ))}
          </div>
        ))}
      </div>

      <div className="sidebar-footer">
        <button className="full-btn" onClick={onOpenLocal}>
          <Icon name="desktop" /> Локальный терминал
        </button>
        <button className="full-btn" onClick={onOpenKeyGen}>
          <Icon name="key" /> Генерация ключей
        </button>
        <button className="full-btn" onClick={onOpenSettings}>
          <Icon name="settings" /> Настройки
        </button>
      </div>
    </aside>
  )
}
