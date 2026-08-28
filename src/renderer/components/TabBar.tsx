import { useEffect, useMemo, useState } from 'react'
import type { Tab, SplitChoice } from '../App'
import type { ServerConfig, Snippet, WorkspaceTool } from '../../shared/types'
import { findLeaf } from '../paneTree'
import { Icon } from './Icon'

interface Props {
  tabs: Tab[]
  activeKey: string | null
  servers: ServerConfig[]
  onSelect: (key: string) => void
  onClose: (key: string) => void
  onNewLocal: () => void
  onToggleSftp: (key: string) => void
  onSetWorkspace: (key: string, tool: WorkspaceTool) => void
  onDetachTab: (key: string) => void
  onRename: (key: string, title: string) => void
  onReorder: (fromKey: string, toKey: string) => void
  onSplit: (tabKey: string, dir: 'row' | 'col', choice: SplitChoice) => void
  broadcast: boolean
  onToggleBroadcast: () => void
}

function SnippetMenu({
  sessionId,
  onClose
}: {
  sessionId: string
  onClose: () => void
}): JSX.Element {
  const [snippets, setSnippets] = useState<Snippet[]>([])
  const [filter, setFilter] = useState('')
  const [adding, setAdding] = useState(false)
  const [newName, setNewName] = useState('')
  const [newCmd, setNewCmd] = useState('')

  useEffect(() => {
    window.api.snippets.list().then(setSnippets)
  }, [])

  const insert = (cmd: string): void => {
    window.api.session.write(sessionId, cmd + '\r')
    onClose()
  }

  const add = async (): Promise<void> => {
    if (!newName.trim() || !newCmd.trim()) return
    const saved = await window.api.snippets.save({ id: '', name: newName.trim(), command: newCmd.trim() })
    setSnippets((prev) => [...prev, saved])
    setNewName('')
    setNewCmd('')
    setAdding(false)
  }

  const remove = async (id: string): Promise<void> => {
    await window.api.snippets.remove(id)
    setSnippets((prev) => prev.filter((s) => s.id !== id))
  }

  const q = filter.toLowerCase()
  const filtered = snippets.filter(
    (s) => !q || s.name.toLowerCase().includes(q) || s.command.toLowerCase().includes(q)
  )

  return (
    <>
      <div className="split-menu-backdrop" onClick={onClose} />
      <div className="snippet-menu">
        <div className="tunnel-menu-header">
          <span className="tunnel-menu-title">Сниппеты</span>
          <button className="mini" onClick={() => setAdding((v) => !v)}>+ Новый</button>
        </div>
        {adding && (
          <div className="snippet-add-form">
            <input
              autoFocus
              placeholder="Название"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => e.key === 'Escape' && setAdding(false)}
            />
            <input
              placeholder="Команда"
              value={newCmd}
              onChange={(e) => setNewCmd(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') void add(); if (e.key === 'Escape') setAdding(false) }}
            />
            <div style={{ display: 'flex', gap: 6, justifyContent: 'flex-end' }}>
              <button className="secondary" style={{ padding: '4px 10px', fontSize: 12 }} onClick={() => setAdding(false)}>Отмена</button>
              <button className="primary" style={{ padding: '4px 10px', fontSize: 12 }} onClick={() => void add()}>Добавить</button>
            </div>
          </div>
        )}
        <input
          className="search"
          style={{ margin: '6px 8px', width: 'calc(100% - 16px)' }}
          placeholder="Поиск…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => e.key === 'Escape' && onClose()}
        />
        <div className="snippet-list">
          {filtered.length === 0 && (
            <div className="hint" style={{ padding: '8px 12px' }}>
              {snippets.length === 0 ? 'Нет сниппетов. Создайте первый.' : 'Ничего не найдено.'}
            </div>
          )}
          {filtered.map((s) => (
            <div key={s.id} className="snippet-row">
              <div className="snippet-info" onClick={() => insert(s.command)}>
                <span className="snippet-name">{s.name}</span>
                <span className="snippet-cmd">{s.command}</span>
              </div>
              <button className="mini danger" title="Удалить" onClick={() => void remove(s.id)}>✕</button>
            </div>
          ))}
        </div>
      </div>
    </>
  )
}

const statusColor: Record<string, string> = {
  connecting: '#e0af68',
  connected: '#9ece6a',
  reconnecting: '#e0af68',
  closed: '#565f89',
  error: '#f7768e'
}

export function TabBar({
  tabs,
  activeKey,
  servers,
  onSelect,
  onClose,
  onNewLocal,
  onToggleSftp,
  onSetWorkspace,
  onDetachTab,
  onRename,
  onReorder,
  onSplit,
  broadcast,
  onToggleBroadcast
}: Props): JSX.Element {
  const active = tabs.find((t) => t.key === activeKey)
  const activeLeaf = active ? findLeaf(active.root, active.activePaneId) : undefined
  const [editingKey, setEditingKey] = useState<string | null>(null)
  const [editValue, setEditValue] = useState('')
  const [draggingKey, setDraggingKey] = useState<string | null>(null)
  const [dragOverKey, setDragOverKey] = useState<string | null>(null)
  const [splitDir, setSplitDir] = useState<'row' | 'col' | null>(null)
  const [splitFilter, setSplitFilter] = useState('')
  const [snippetOpen, setSnippetOpen] = useState(false)
  const [logging, setLogging] = useState(false)

  const activeSessionId = activeLeaf?.sessionId

  useEffect(() => {
    if (activeSessionId) window.api.session.logStatus(activeSessionId).then(setLogging)
    else setLogging(false)
  }, [activeSessionId])

  const toggleLog = async (): Promise<void> => {
    if (!activeSessionId || !active) return
    const r = await window.api.session.logToggle(activeSessionId, active.title)
    setLogging(r.logging)
  }

  const commit = (): void => {
    if (editingKey && editValue.trim()) onRename(editingKey, editValue.trim())
    setEditingKey(null)
  }

  const openSplit = (dir: 'row' | 'col'): void => {
    setSplitFilter('')
    setSplitDir((cur) => (cur === dir ? null : dir))
  }
  const choose = (choice: SplitChoice): void => {
    if (active && splitDir) onSplit(active.key, splitDir, choice)
    setSplitDir(null)
  }

  const splitGroups = useMemo(() => {
    const q = splitFilter.trim().toLowerCase()
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
  }, [servers, splitFilter])
  return (
    <div className="tabbar">
      <div className="tabs">
        <div className="tabs-scroll">
        {tabs.map((t) => {
          const leaf = findLeaf(t.root, t.activePaneId)
          return (
          <div
            key={t.key}
            className={
              'tab' +
              (t.key === activeKey ? ' active' : '') +
              (t.key === draggingKey ? ' dragging' : '') +
              (t.key === dragOverKey && t.key !== draggingKey ? ' drag-over' : '')
            }
            onClick={() => onSelect(t.key)}
            title={leaf?.statusMsg || t.title}
            draggable={editingKey !== t.key}
            onDragStart={(e) => {
              e.dataTransfer.setData('tab', t.key)
              setDraggingKey(t.key)
            }}
            onDragEnd={() => {
              setDraggingKey(null)
              setDragOverKey(null)
            }}
            onDragOver={(e) => {
              e.preventDefault()
              if (dragOverKey !== t.key) setDragOverKey(t.key)
            }}
            onDrop={(e) => {
              e.preventDefault()
              const from = e.dataTransfer.getData('tab')
              if (from) onReorder(from, t.key)
              setDragOverKey(null)
            }}
          >
            <span
              className="status-dot"
              style={{
                background:
                  t.kind === 'editor'
                    ? t.editorDirty
                      ? '#e0af68'
                      : '#9ece6a'
                    : statusColor[leaf?.status ?? 'connecting']
              }}
            />
            <span className="tab-icon">
              <Icon name={t.kind === 'editor' ? 'editor' : leaf?.kind === 'local' ? 'desktop' : 'server'} size={13} />
            </span>
            {editingKey === t.key ? (
              <input
                className="tab-rename"
                autoFocus
                value={editValue}
                onChange={(e) => setEditValue(e.target.value)}
                onBlur={commit}
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') commit()
                  if (e.key === 'Escape') setEditingKey(null)
                }}
              />
            ) : (
              <span
                className="tab-title"
                onDoubleClick={(e) => {
                  e.stopPropagation()
                  setEditingKey(t.key)
                  setEditValue(t.title)
                }}
              >
                {t.title}
              </span>
            )}
            <button
              className="tab-close"
              onClick={(e) => {
                e.stopPropagation()
                onClose(t.key)
              }}
            >
              <Icon name="close" size={12} />
            </button>
          </div>
          )
        })}
        </div>
        <button className="tab-new" title="Новый локальный терминал" onClick={onNewLocal}>
          <Icon name="plus" />
        </button>
      </div>

      <div className="tabbar-right">
        {tabs.length > 1 && (
          <button
            className={'tool-btn' + (broadcast ? ' on' : '')}
            title="Broadcast: дублировать ввод в панели текущей вкладки"
            onClick={onToggleBroadcast}
          >
            <Icon name="broadcast" />
          </button>
        )}
        {active && active.kind === 'terminal' && (
          <div className="split-control">
            <button
              className={'tool-btn' + (splitDir === 'row' ? ' on' : '')}
              title="Разделить по вертикали (панели рядом)"
              onClick={() => openSplit('row')}
            >
              <Icon name="split-h" />
            </button>
            <button
              className={'tool-btn' + (splitDir === 'col' ? ' on' : '')}
              title="Разделить по горизонтали (панели друг над другом)"
              onClick={() => openSplit('col')}
            >
              <Icon name="split-v" />
            </button>
            {splitDir && (
              <>
                <div className="split-menu-backdrop" onClick={() => setSplitDir(null)} />
                <div className="split-menu">
                  <div className="split-menu-title">Что открыть в новой панели?</div>
                  <input
                    className="search"
                    autoFocus
                    placeholder="Поиск серверов…"
                    value={splitFilter}
                    onChange={(e) => setSplitFilter(e.target.value)}
                    onKeyDown={(e) => e.key === 'Escape' && setSplitDir(null)}
                  />
                  <div className="split-menu-list">
                    <button className="split-menu-item" onClick={() => choose({ kind: 'local' })}>
                      <span className="tab-icon"><Icon name="desktop" size={14} /></span>
                      <span className="split-menu-name">Локальный терминал</span>
                    </button>
                    {splitGroups.map(([group, items]) => (
                      <div key={group} className="split-menu-group">
                        <div className="group-title">{group}</div>
                        {items.map((s) => (
                          <button
                            key={s.id}
                            className="split-menu-item"
                            onClick={() => choose({ kind: 'ssh', serverId: s.id, title: s.name })}
                          >
                            <span className="dot" style={{ background: s.color || '#7aa2f7' }} />
                            <span className="split-menu-name">{s.name}</span>
                            <span className="split-menu-host">
                              {s.username}@{s.host}
                            </span>
                          </button>
                        ))}
                      </div>
                    ))}
                    {servers.length === 0 && <div className="hint">Нет сохранённых серверов.</div>}
                  </div>
                </div>
              </>
            )}
          </div>
        )}
        {active && activeLeaf?.sessionId && <span className="tool-sep" />}
        {active && activeLeaf?.sessionId && (
          <div className="split-control">
            <button
              className={'tool-btn' + (snippetOpen ? ' on' : '')}
              title="Сниппеты — быстрая вставка команд"
              onClick={() => setSnippetOpen((v) => !v)}
            >
              <Icon name="snippets" />
            </button>
            {snippetOpen && (
              <SnippetMenu
                sessionId={activeLeaf.sessionId}
                onClose={() => setSnippetOpen(false)}
              />
            )}
          </div>
        )}
        {active && activeLeaf?.sessionId && (
          <button
            className={'tool-btn' + (logging ? ' on' : '')}
            title={logging ? 'Логирование включено — нажмите, чтобы остановить' : 'Логировать вывод сессии в файл'}
            onClick={toggleLog}
          >
            <Icon name={logging ? 'log-on' : 'log'} />
          </button>
        )}
        {active && activeLeaf?.sessionId && activeLeaf.status === 'connected' && (
          <>
            <span className="tool-sep" />
            <button
              className="tool-btn"
              title="Открепить вкладку в отдельное окно"
              onClick={() => onDetachTab(active.key)}
            >
              <Icon name="external" />
            </button>
            {activeLeaf.kind === 'ssh' && (
              <>
                <button
                  className={'tool-btn' + (active.workspace === 'tunnels' ? ' on' : '')}
                  title="Проброс портов (туннели)"
                  onClick={() => onSetWorkspace(active.key, active.workspace === 'tunnels' ? 'terminal' : 'tunnels')}
                >
                  <Icon name="tunnel" />
                </button>
                <button
                  className={'tool-btn' + (active.sftpOpen ? ' on' : '')}
                  title="Файловый менеджер (SFTP)"
                  onClick={() => onToggleSftp(active.key)}
                >
                  <Icon name="folder" />
                </button>
              </>
            )}
          </>
        )}
      </div>
    </div>
  )
}
