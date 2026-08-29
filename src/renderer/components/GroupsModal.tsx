import { useMemo, useState } from 'react'
import type { ServerConfig } from '../../shared/types'
import { Icon } from './Icon'

interface Props {
  groups: string[]
  servers: ServerConfig[]
  onClose: () => void
  /** Переименование: меняет имя в порядке групп и у всех серверов этой группы. */
  onRename: (from: string, to: string) => void
  onDelete: (group: string) => void
  onCreate: (name: string) => void
  /** Перенос сервера в группу; пустая строка — вынести из групп. */
  onAssign: (serverId: string, group: string) => void
  onMove: (group: string, dir: -1 | 1) => void
}

export function GroupsModal({
  groups,
  servers,
  onClose,
  onRename,
  onDelete,
  onCreate,
  onAssign,
  onMove
}: Props): JSX.Element {
  const [selected, setSelected] = useState<string | null>(groups[0] ?? null)
  const [renameTo, setRenameTo] = useState('')
  const [newName, setNewName] = useState('')

  const members = useMemo(
    () => servers.filter((s) => (s.group?.trim() || '') === (selected ?? '')),
    [servers, selected]
  )
  const outsiders = useMemo(
    () => servers.filter((s) => (s.group?.trim() || '') !== (selected ?? '')),
    [servers, selected]
  )

  const applyRename = (): void => {
    const to = renameTo.trim()
    if (!selected || !to || to === selected) return
    if (groups.some((g) => g === to)) {
      alert(`Группа «${to}» уже есть`)
      return
    }
    onRename(selected, to)
    setSelected(to)
    setRenameTo('')
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal groups-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Настройки групп</h2>

        <div className="groups-layout">
          <div className="groups-col">
            <div className="groups-col-title">Группы</div>
            <div className="groups-list">
              {groups.length === 0 && <div className="agent-hint">Групп пока нет.</div>}
              {groups.map((g, i) => (
                <div
                  key={g}
                  className={'groups-row' + (g === selected ? ' selected' : '')}
                  onClick={() => {
                    setSelected(g)
                    setRenameTo('')
                  }}
                >
                  <span className="groups-row-name">{g}</span>
                  <span className="groups-row-count">
                    {servers.filter((s) => (s.group?.trim() || '') === g).length}
                  </span>
                  <button
                    className="mini"
                    title="Выше"
                    disabled={i === 0}
                    onClick={(e) => {
                      e.stopPropagation()
                      onMove(g, -1)
                    }}
                  >
                    ↑
                  </button>
                  <button
                    className="mini"
                    title="Ниже"
                    disabled={i === groups.length - 1}
                    onClick={(e) => {
                      e.stopPropagation()
                      onMove(g, 1)
                    }}
                  >
                    ↓
                  </button>
                </div>
              ))}
            </div>

            <div className="row" style={{ marginTop: 10 }}>
              <input
                placeholder="Название новой группы"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key !== 'Enter') return
                  const n = newName.trim()
                  if (!n) return
                  onCreate(n)
                  setSelected(n)
                  setNewName('')
                }}
              />
              <button
                className="mini"
                onClick={() => {
                  const n = newName.trim()
                  if (!n) return
                  onCreate(n)
                  setSelected(n)
                  setNewName('')
                }}
              >
                Создать
              </button>
            </div>
          </div>

          <div className="groups-col">
            {selected ? (
              <>
                <div className="groups-col-title">Группа «{selected}»</div>

                <label>
                  Переименовать
                  <div className="row">
                    <input
                      value={renameTo}
                      placeholder={selected}
                      onChange={(e) => setRenameTo(e.target.value)}
                      onKeyDown={(e) => e.key === 'Enter' && applyRename()}
                    />
                    <button className="mini" onClick={applyRename} disabled={!renameTo.trim()}>
                      Применить
                    </button>
                  </div>
                </label>

                <div className="groups-col-title" style={{ marginTop: 12 }}>
                  В группе ({members.length})
                </div>
                <div className="groups-members">
                  {members.length === 0 && <div className="agent-hint">Пока пусто.</div>}
                  {members.map((s) => (
                    <div key={s.id} className="groups-member">
                      <span className="dot" style={{ background: s.color || '#7aa2f7' }} />
                      <span className="groups-member-name">{s.name}</span>
                      <button className="mini" title="Убрать из группы" onClick={() => onAssign(s.id, '')}>
                        ✕
                      </button>
                    </div>
                  ))}
                </div>

                <div className="groups-col-title" style={{ marginTop: 12 }}>
                  Добавить сервер
                </div>
                <div className="groups-members">
                  {outsiders.length === 0 && <div className="agent-hint">Все серверы уже здесь.</div>}
                  {outsiders.map((s) => (
                    <div key={s.id} className="groups-member">
                      <span className="dot" style={{ background: s.color || '#7aa2f7' }} />
                      <span className="groups-member-name">{s.name}</span>
                      <span className="groups-member-where">{s.group?.trim() || 'без группы'}</span>
                      <button
                        className="mini"
                        title="Добавить в группу"
                        onClick={() => onAssign(s.id, selected)}
                      >
                        <Icon name="plus" size={12} />
                      </button>
                    </div>
                  ))}
                </div>

                <button
                  className="mini danger"
                  style={{ marginTop: 14 }}
                  onClick={() => {
                    if (!confirm(`Удалить группу «${selected}»? Серверы останутся, но окажутся без группы.`))
                      return
                    onDelete(selected)
                    setSelected(null)
                  }}
                >
                  Удалить группу
                </button>
              </>
            ) : (
              <div className="agent-hint">Выберите группу слева или создайте новую.</div>
            )}
          </div>
        </div>

        <div className="modal-actions">
          <button className="primary" onClick={onClose}>
            Готово
          </button>
        </div>
      </div>
    </div>
  )
}
