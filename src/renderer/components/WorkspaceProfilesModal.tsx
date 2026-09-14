import { useEffect, useMemo, useState } from 'react'
import type { SerializedTab, ServerConfig, WorkspaceProfile } from '../../shared/types'
import { errText } from '../errText'
import { nameProblem, profileInfo, profileSummary, withoutMissing } from '../workspaceProfiles'

interface Props {
  servers: ServerConfig[]
  /** Текущие вкладки в сохраняемом виде. */
  snapshot: () => SerializedTab[]
  /** Открыть вкладки рядом с текущими или вместо них. `false` - человек передумал. */
  onOpen: (tabs: SerializedTab[], replace: boolean) => boolean
  onClose: () => void
}

/**
 * Именованные наборы вкладок.
 *
 * Сохраняется то же, что восстанавливается при запуске: вкладки терминалов, разбиение на
 * панели и открытый инструмент. Редакторы файлов и утилиты в профиль не попадают - первые
 * держат несохранённую правку, вторые одни на приложение.
 */
export function WorkspaceProfilesModal({ servers, snapshot, onOpen, onClose }: Props): JSX.Element {
  const [profiles, setProfiles] = useState<WorkspaceProfile[]>([])
  const [name, setName] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(true)
  const current = useMemo(() => snapshot(), [snapshot])

  const reload = async (): Promise<void> => {
    try {
      const list = await window.api.workspaces.list()
      setProfiles([...list].sort((a, b) => a.name.localeCompare(b.name)))
    } catch (e) {
      setError(errText(e))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void reload()
  }, [])

  const problem = name.trim() ? nameProblem(name, profiles) : null

  const saveNew = async (): Promise<void> => {
    const why = nameProblem(name, profiles)
    if (why) {
      setError(why)
      return
    }
    try {
      await window.api.workspaces.save({ name: name.trim(), tabs: current, savedAt: Date.now() })
      setName('')
      setError('')
      await reload()
    } catch (e) {
      setError(errText(e))
    }
  }

  const overwrite = async (p: WorkspaceProfile): Promise<void> => {
    if (!confirm(`Заменить содержимое «${p.name}» текущими вкладками (${current.length})?`)) return
    try {
      await window.api.workspaces.save({ ...p, tabs: current, savedAt: Date.now() })
      await reload()
    } catch (e) {
      setError(errText(e))
    }
  }

  const remove = async (p: WorkspaceProfile): Promise<void> => {
    if (!p.id || !confirm(`Удалить профиль «${p.name}»? Открытые вкладки это не закроет.`)) return
    try {
      await window.api.workspaces.remove(p.id)
      await reload()
    } catch (e) {
      setError(errText(e))
    }
  }

  const open = (p: WorkspaceProfile, replace: boolean): void => {
    const tabs = withoutMissing(p.tabs, servers)
    if (tabs.length === 0) {
      setError(`В «${p.name}» не осталось вкладок: все его серверы удалены`)
      return
    }
    if (replace && !confirm(`Закрыть все текущие вкладки и открыть «${p.name}»?`)) return
    if (onOpen(tabs, replace)) onClose()
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
        <h2>Профили рабочего пространства</h2>
        <div className="hint">
          Профиль запоминает вкладки терминалов, их разбиение на панели и открытые инструменты. Паролей в нём нет -
          только ссылки на серверы.
        </div>

        <div className="profile-save">
          <input
            value={name}
            placeholder="Название, например «Продакшн»"
            onChange={(e) => {
              setName(e.target.value)
              setError('')
            }}
            onKeyDown={(e) => e.key === 'Enter' && current.length > 0 && !problem && void saveNew()}
          />
          <button className="primary" disabled={current.length === 0 || !name.trim() || !!problem} onClick={() => void saveNew()}>
            Сохранить текущие ({current.length})
          </button>
        </div>
        {current.length === 0 && <div className="hint">Открытых вкладок терминалов нет - сохранять нечего.</div>}
        {(problem || error) && <div className="settings-msg err">{problem ?? error}</div>}

        <div className="profile-list">
          {loading && <div className="hint">Загружаю…</div>}
          {!loading && profiles.length === 0 && <div className="hint">Профилей пока нет.</div>}
          {profiles.map((p) => {
            const info = profileInfo(p, servers)
            return (
              <div key={p.id ?? p.name} className="profile-row">
                <div className="profile-main">
                  <div className="profile-name">{p.name}</div>
                  <div className="profile-summary">
                    {profileSummary(info)} · {new Date(p.savedAt).toLocaleString('ru-RU')}
                  </div>
                </div>
                <div className="profile-actions">
                  <button className="mini" onClick={() => open(p, false)} title="Открыть вкладки профиля рядом с текущими">
                    Открыть
                  </button>
                  <button className="mini" onClick={() => open(p, true)} title="Закрыть текущие вкладки и открыть профиль">
                    Вместо текущих
                  </button>
                  <button
                    className="mini"
                    disabled={current.length === 0}
                    onClick={() => void overwrite(p)}
                    title="Сохранить в этот профиль текущие вкладки"
                  >
                    Перезаписать
                  </button>
                  <button className="mini danger" onClick={() => void remove(p)}>
                    Удалить
                  </button>
                </div>
              </div>
            )
          })}
        </div>

        <div className="modal-actions">
          <button onClick={onClose}>Закрыть</button>
        </div>
      </div>
    </div>
  )
}
