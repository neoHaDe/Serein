import { useEffect, useMemo, useState } from 'react'
import { errText } from '../errText'
import {
  KIND_LABEL,
  KIND_ORDER,
  changedSincePlan,
  countKinds,
  toUpload,
  uploadSteps,
  type SyncItem,
  type SyncPlan
} from '../folderSync'

interface Props {
  sessionId: string
  localDir: string
  remoteDir: string
  onClose: () => void
}

/** Сколько строк показываем. Остальное считается и заливается, но не рисуется разом. */
const SHOW_LIMIT = 500

function fmtBytes(n: number | null | undefined): string {
  if (n === null || n === undefined) return '-'
  if (n < 1024) return `${n} Б`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} КиБ`
  return `${(n / 1024 / 1024).toFixed(1)} МиБ`
}

function sizeText(it: SyncItem): string {
  if (it.kind === 'new') return fmtBytes(it.localSize)
  if (it.kind === 'remoteOnly') return fmtBytes(it.remoteSize)
  if (it.localSize === it.remoteSize) return fmtBytes(it.localSize)
  return `${fmtBytes(it.localSize)} → ${fmtBytes(it.remoteSize)}`
}

/**
 * Сравнение открытой здесь папки с открытой на сервере и заливка изменённого.
 *
 * Сравнение само по себе - пробный прогон: ничего не пишет ни с одной стороны, и человек
 * видит поимённо, что будет залито, до того, как нажмёт кнопку. Лишнее на сервере не
 * удаляется вовсе - это обещание узкой синхронизации, а не недоделка.
 */
export function FolderSyncModal({ sessionId, localDir, remoteDir, onClose }: Props): JSX.Element {
  const [plan, setPlan] = useState<SyncPlan | null>(null)
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(true)
  const [includeRemoteNewer, setIncludeRemoteNewer] = useState(false)
  const [includeUnsure, setIncludeUnsure] = useState(false)
  const [showSame, setShowSame] = useState(false)
  const [starting, setStarting] = useState(false)

  const load = async (): Promise<void> => {
    setLoading(true)
    setError('')
    try {
      setPlan(await window.api.sftp.compare(sessionId, localDir, remoteDir))
    } catch (e) {
      setError(errText(e))
      setPlan(null)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void load()
    // Сравниваем заново только при смене самих папок: остальное - кнопкой.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId, localDir, remoteDir])

  const counts = plan ? countKinds(plan.items) : null
  const steps = plan ? uploadSteps(plan, includeRemoteNewer, includeUnsure) : null
  const visible = useMemo(
    () => (plan ? plan.items.filter((it) => showSame || it.kind !== 'same') : []),
    [plan, showSame]
  )

  const upload = async (): Promise<void> => {
    if (!plan || !steps || steps.files === 0) return
    if (!confirm(`Залить ${steps.files} файлов в ${plan.remoteRoot}?`)) return
    setStarting(true)
    setError('')
    try {
      // С момента сравнения могли пройти минуты, а согласие было на те версии файлов на
      // сервере. Сравниваем заново и заливаем, только если заливаемое там не менялось. Строгой
      // гарантии SFTP не даёт - между проверкой и записью остаются секунды, - но окно больше
      // не длится, пока открыто это окно.
      const fresh = await window.api.sftp.compare(sessionId, localDir, remoteDir)
      const moved = changedSincePlan(toUpload(plan.items, includeRemoteNewer, includeUnsure), fresh)
      if (moved.length > 0) {
        setPlan(fresh)
        setError(
          `После сравнения на сервере изменились файлы (${moved.length}): ${moved.slice(0, 5).join(', ')}` +
            `${moved.length > 5 ? '…' : ''}. Сравнение обновлено - проверьте и залейте снова.`
        )
        setStarting(false)
        return
      }
      for (const dir of steps.mkdirs) {
        try {
          await window.api.sftp.mkdir(sessionId, dir)
        } catch {
          // Каталог мог появиться между сравнением и заливкой - это не повод останавливаться.
        }
      }
      // Сама заливка идёт очередью передач панели: прогресс, отмена и ошибки по каждому
      // файлу видны там, поэтому окно закрывается сразу, а не ждёт последнего файла.
      for (const g of steps.groups) void window.api.sftp.uploadPaths(sessionId, g.remoteDir, g.localPaths)
      onClose()
    } catch (e) {
      setError(errText(e))
      setStarting(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && !starting && onClose()}>
      <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
        <h2>Сравнение папок</h2>
        <div className="sync-roots">
          <div>
            <span>Здесь</span>
            <code>{localDir}</code>
          </div>
          <div>
            <span>На сервере</span>
            <code>{plan?.remoteRoot ?? remoteDir}</code>
          </div>
        </div>

        {loading && <div className="hint">Сравниваю…</div>}
        {error && <div className="settings-msg err">{error}</div>}

        {plan && counts && !loading && (
          <>
            {!plan.timeKnown && (
              <div className="hint">
                Файлы на этом сервере идут по SCP, а там время правки не узнать. Файлы одного размера помечены
                «не определить»: одинаковое содержимое этим не доказано.
              </div>
            )}
            <div className="sync-counts">
              {KIND_ORDER.map((k) => (
                <span key={k} className={'sync-count ' + k}>
                  {KIND_LABEL[k]}: {counts[k]}
                </span>
              ))}
            </div>
            <div className="sync-list">
              {visible.slice(0, SHOW_LIMIT).map((it) => (
                <div key={it.rel} className={'sync-row ' + it.kind}>
                  <span className="sync-kind">{KIND_LABEL[it.kind]}</span>
                  <span className="sync-rel">{it.rel}</span>
                  <span className="sync-size">{sizeText(it)}</span>
                </div>
              ))}
              {visible.length === 0 && <div className="hint">Отличий нет - заливать нечего.</div>}
              {visible.length > SHOW_LIMIT && (
                <div className="hint">
                  Показаны первые {SHOW_LIMIT} из {visible.length}.
                </div>
              )}
            </div>
            {plan.refused.length > 0 && (
              <details className="sync-refused">
                <summary>Не сравнивались: {plan.refused.length}</summary>
                {plan.refused.map((r) => (
                  <div key={r.rel + r.why}>
                    {r.rel} - {r.why}
                  </div>
                ))}
              </details>
            )}
            <div className="sync-options">
              <label className="checkbox-row">
                <input type="checkbox" checked={showSame} onChange={(e) => setShowSame(e.target.checked)} />
                Показывать совпадающие
              </label>
              {counts.remoteNewer > 0 && (
                <label className="checkbox-row">
                  <input
                    type="checkbox"
                    checked={includeRemoteNewer}
                    onChange={(e) => setIncludeRemoteNewer(e.target.checked)}
                  />
                  Заливать и те, что на сервере новее ({counts.remoteNewer}) - правка на сервере будет затёрта
                </label>
              )}
              {counts.unsure > 0 && (
                <label className="checkbox-row">
                  <input type="checkbox" checked={includeUnsure} onChange={(e) => setIncludeUnsure(e.target.checked)} />
                  Заливать и те, что не определить ({counts.unsure}) - размер тот же, различие не проверить
                </label>
              )}
            </div>
            <div className="hint">Файлы, которых здесь нет, на сервере не удаляются.</div>
          </>
        )}

        <div className="modal-actions">
          <button onClick={onClose} disabled={starting}>
            Закрыть
          </button>
          <button onClick={() => void load()} disabled={loading || starting}>
            Сравнить заново
          </button>
          <button
            className="primary"
            disabled={!steps || steps.files === 0 || loading || starting}
            onClick={() => void upload()}
          >
            {starting ? 'Запускаю…' : `Залить ${steps?.files ?? 0}`}
          </button>
        </div>
      </div>
    </div>
  )
}
