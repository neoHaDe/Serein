import { useCallback, useEffect, useState } from 'react'
import type { SftpEntry } from '../../shared/types'
import { errText } from '../errText'
import { Icon } from './Icon'

/**
 * Выбор файла на сервере обзором по SFTP.
 *
 * Заведён потому, что путь, набранный руками, - это способ ошибиться дважды: опечататься
 * в имени и не заметить, что файла там вовсе нет. Ответ «не удалось прочитать» в такой
 * ситуации ничего не объясняет: непонятно, файла нет, прав нет или буква не та.
 *
 * Нарочно только выбор, без переименований и удаления: для работы с файлами есть полная
 * панель, а здесь нужен один ответ - какой путь взять.
 */
interface Props {
  sessionId: string
  /** С какого каталога начать. Пусто - домашний каталог пользователя. */
  startPath?: string
  onPick: (path: string) => void
  onClose: () => void
}

/** Склейка пути без двойных слэшей: `/` + `etc` должно дать `/etc`, а не `//etc`. */
function join(dir: string, name: string): string {
  if (dir === '/' || dir === '') return `/${name}`
  return `${dir.replace(/\/+$/, '')}/${name}`
}

/** На уровень вверх. Из корня выйти некуда - там и остаёмся. */
function parent(dir: string): string {
  const trimmed = dir.replace(/\/+$/, '')
  const i = trimmed.lastIndexOf('/')
  if (i <= 0) return '/'
  return trimmed.slice(0, i)
}

export function RemoteFilePicker({ sessionId, startPath, onPick, onClose }: Props): JSX.Element {
  const [path, setPath] = useState(startPath || '.')
  const [entries, setEntries] = useState<SftpEntry[]>([])
  const [busy, setBusy] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(
    async (p: string) => {
      setBusy(true)
      setError(null)
      try {
        const res = await window.api.sftp.list(sessionId, p)
        // Путь берём из ответа, а не из запроса: сервер разворачивает `.` и `~` в
        // настоящий, и показывать надо именно его - иначе кнопка «вверх» пойдёт не туда.
        setPath(res.path)
        // Каталоги наверх, дальше по алфавиту: так список читается, а не просматривается.
        setEntries(
          [...res.entries].sort((a, b) => {
            const da = a.type === 'dir' ? 0 : 1
            const db = b.type === 'dir' ? 0 : 1
            return da - db || a.name.localeCompare(b.name, 'ru')
          })
        )
      } catch (e) {
        setError(errText(e))
      } finally {
        setBusy(false)
      }
    },
    [sessionId]
  )

  useEffect(() => {
    void load(startPath || '.')
  }, [load, startPath])

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal picker-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Выбрать файл на сервере</h2>

        <div className="picker-path">
          <button className="mini" title="На уровень вверх" onClick={() => void load(parent(path))}>
            <Icon name="up-dir" size={14} />
          </button>
          <input
            value={path}
            onChange={(e) => setPath(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && void load(path)}
            spellCheck={false}
          />
          <button className="mini" title="Перейти" onClick={() => void load(path)}>
            <Icon name="refresh" size={14} />
          </button>
        </div>

        {error && <p className="tools-error">{error}</p>}

        <div className="picker-list">
          {busy && <div className="hint picker-empty">Читаю каталог…</div>}
          {!busy && entries.length === 0 && !error && (
            <div className="hint picker-empty">Каталог пуст</div>
          )}
          {!busy &&
            entries.map((e) => {
              const dir = e.type === 'dir'
              return (
                <button
                  key={e.name}
                  className="picker-row"
                  onDoubleClick={() => dir && void load(join(path, e.name))}
                  onClick={() => (dir ? void load(join(path, e.name)) : onPick(join(path, e.name)))}
                  title={dir ? 'Открыть каталог' : 'Выбрать этот файл'}
                >
                  <Icon name={dir ? 'folder' : 'file'} size={14} />
                  <span className="picker-name">{e.name}</span>
                  {!dir && <span className="picker-size mono">{fmtSize(e.size)}</span>}
                </button>
              )
            })}
        </div>

        <div className="modal-actions">
          <button onClick={onClose}>Отмена</button>
        </div>
      </div>
    </div>
  )
}

/** Размер файла человеческими единицами. Байты у конфигов и логов читаются плохо. */
function fmtSize(n: number): string {
  if (n < 1024) return `${n} Б`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} КБ`
  return `${(n / 1024 / 1024).toFixed(1)} МБ`
}
