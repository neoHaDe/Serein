import { useEffect, useMemo, useRef, useState } from 'react'
import type { ServerConfig } from '../../shared/types'
import { errText } from '../errText'
import {
  CHECK_LABEL,
  DOCKER_ACTIONS,
  SERVICE_ACTIONS,
  STEP_KINDS,
  STEP_KIND_LABEL,
  STEP_STATE_LABEL,
  TEMPLATES,
  WHEN_LABEL,
  applyProgress,
  effectiveServers,
  initialValues,
  moveStep,
  newStep,
  progressFromReport,
  promptVariables,
  runSummary,
  stepLabel,
  taskProblem,
  withStepIds,
  type CheckKind,
  type RunReport,
  type ServerProgress,
  type StepKind,
  type StepWhen,
  type TaskDef,
  type TaskProfile,
  type TaskStep,
  type TaskVariable
} from '../taskModel'

interface Props {
  servers: ServerConfig[]
  onClose: () => void
}

/** Задачи выполняются там, где есть оболочка. */
function runnable(s: ServerConfig): boolean {
  return s.connection !== 'serial' && s.connection !== 'telnet' && s.connection !== 'raw'
}

const emptyTask = (): TaskDef => ({ name: 'Новая задача', steps: [], serverIds: [], concurrency: 4 })

const SERVER_STATE_LABEL: Record<string, string> = {
  connecting: 'подключается',
  running: 'выполняется',
  done: 'готово',
  failed: 'ошибка',
  skipped: 'пропущен',
  cancelled: 'остановлен',
  problem: 'есть проблемы'
}

function StepFields({ step, onChange }: { step: TaskStep; onChange: (p: Partial<TaskStep>) => void }): JSX.Element {
  const pick = async (key: 'localPath', directory: boolean, title: string): Promise<void> => {
    const [p] = await window.api.files.pick({ title, directory, multiple: false })
    if (p) onChange({ [key]: p })
  }
  const localRow = (label: string, allowFile: boolean): JSX.Element => (
    <label className="task-field wide">
      {label}
      <div className="task-path">
        <input value={step.localPath ?? ''} onChange={(e) => onChange({ localPath: e.target.value })} spellCheck={false} />
        <button className="mini" onClick={() => void pick('localPath', true, 'Своя папка')}>
          Папка…
        </button>
        {allowFile && (
          <button className="mini" onClick={() => void pick('localPath', false, 'Свой файл')}>
            Файл…
          </button>
        )}
      </div>
    </label>
  )
  const remoteRow = (label: string, placeholder: string): JSX.Element => (
    <label className="task-field wide">
      {label}
      <input
        value={step.remotePath ?? ''}
        placeholder={placeholder}
        onChange={(e) => onChange({ remotePath: e.target.value })}
        spellCheck={false}
      />
    </label>
  )

  switch (step.kind) {
    case 'command':
      return (
        <label className="task-field wide">
          Команда
          <textarea
            rows={3}
            value={step.command ?? ''}
            placeholder="apt-get update && apt-get -y upgrade"
            spellCheck={false}
            onChange={(e) => onChange({ command: e.target.value })}
          />
        </label>
      )
    case 'upload':
      return (
        <>
          {localRow('Что залить (файл или папка)', true)}
          {remoteRow('В каталог на сервере', '/var/www')}
        </>
      )
    case 'sync':
      return (
        <>
          {localRow('Своя папка', false)}
          {remoteRow('Папка на сервере', '/var/www/site')}
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={!!step.includeRemoteNewer}
              onChange={(e) => onChange({ includeRemoteNewer: e.target.checked })}
            />
            Заливать и файлы, которые на сервере новее - правка на сервере будет затёрта
          </label>
        </>
      )
    case 'download':
      return (
        <>
          {remoteRow('Что скачать с сервера (файл или папка)', '/var/log/nginx')}
          {localRow('В свою папку', false)}
          <div className="hint">С нескольких серверов - по подпапкам с именами серверов.</div>
        </>
      )
    case 'service':
      return (
        <>
          <label className="task-field">
            Служба
            <input value={step.service ?? ''} placeholder="nginx" onChange={(e) => onChange({ service: e.target.value })} />
          </label>
          <label className="task-field">
            Действие
            <select value={step.action ?? 'restart'} onChange={(e) => onChange({ action: e.target.value })}>
              {SERVICE_ACTIONS.map((a) => (
                <option key={a}>{a}</option>
              ))}
            </select>
          </label>
        </>
      )
    case 'docker':
      return (
        <>
          <label className="task-field">
            Контейнер
            <input value={step.container ?? ''} placeholder="web" onChange={(e) => onChange({ container: e.target.value })} />
          </label>
          <label className="task-field">
            Действие
            <select value={step.action ?? 'restart'} onChange={(e) => onChange({ action: e.target.value })}>
              {DOCKER_ACTIONS.map((a) => (
                <option key={a}>{a}</option>
              ))}
            </select>
          </label>
        </>
      )
    case 'healthcheck':
      return (
        <>
          <label className="task-field">
            Условие
            <select value={step.check ?? 'http'} onChange={(e) => onChange({ check: e.target.value as CheckKind })}>
              {(Object.keys(CHECK_LABEL) as CheckKind[]).map((c) => (
                <option key={c} value={c}>
                  {CHECK_LABEL[c]}
                </option>
              ))}
            </select>
          </label>
          <label className="task-field wide">
            {step.check === 'command' ? 'Команда' : step.check === 'port' ? 'Адрес:порт' : 'Адрес'}
            <input
              value={step.target ?? ''}
              spellCheck={false}
              placeholder={step.check === 'command' ? 'systemctl is-active nginx' : step.check === 'port' ? '127.0.0.1:8080' : 'http://127.0.0.1/health'}
              onChange={(e) => onChange({ target: e.target.value })}
            />
          </label>
          <label className="task-field">
            Попыток
            <input
              type="number"
              min={1}
              max={60}
              value={step.attempts ?? 3}
              onChange={(e) => onChange({ attempts: Math.max(1, Number(e.target.value) || 1) })}
            />
          </label>
          <label className="task-field">
            Пауза между, с
            <input
              type="number"
              min={1}
              max={600}
              value={step.intervalSec ?? 5}
              onChange={(e) => onChange({ intervalSec: Math.max(1, Number(e.target.value) || 1) })}
            />
          </label>
          <div className="hint">Проверка идёт с самого сервера: адрес - такой, каким его видит сервер.</div>
        </>
      )
  }
}

/**
 * Задачи: шаги по порядку на выбранных серверах.
 *
 * Пробный прогон стоит рядом с запуском не для красоты: это место, где одно нажатие меняет
 * состояние нескольких машин разом, и увидеть план - что будет залито, есть ли служба - до
 * запуска дешевле, чем разбирать последствия после.
 */
export function TasksModal({ servers, onClose }: Props): JSX.Element {
  const [tasks, setTasks] = useState<TaskDef[]>([])
  const [runs, setRuns] = useState<RunReport[]>([])
  const [draft, setDraft] = useState<TaskDef | null>(null)
  const [dirty, setDirty] = useState(false)
  const [error, setError] = useState('')
  const [note, setNote] = useState('')
  const [addKind, setAddKind] = useState<StepKind>('command')
  const [serverFilter, setServerFilter] = useState('')
  const [running, setRunning] = useState(false)
  const [dryRun, setDryRun] = useState(false)
  const [progress, setProgress] = useState<ServerProgress[]>([])
  const [report, setReport] = useState<RunReport | null>(null)
  const [runProfile, setRunProfile] = useState('')
  // Окно ввода значений перед запуском: какой прогон и что уже введено.
  const [asking, setAsking] = useState<{ dry: boolean; values: Record<string, string> } | null>(null)
  const runIdRef = useRef<string | null>(null)

  const load = async (): Promise<void> => {
    try {
      const [list, history] = await Promise.all([window.api.tasks.list(), window.api.tasks.runs()])
      setTasks([...list].sort((a, b) => a.name.localeCompare(b.name)))
      setRuns(history)
    } catch (e) {
      setError(errText(e))
    }
  }

  useEffect(() => {
    void load()
  }, [])

  const choices = useMemo(() => {
    const q = serverFilter.trim().toLowerCase()
    return servers
      .filter(runnable)
      .filter((s) => !q || s.name.toLowerCase().includes(q) || (s.host ?? '').toLowerCase().includes(q))
  }, [servers, serverFilter])

  const history = useMemo(
    () => (draft?.id ? runs.filter((r) => r.taskId === draft.id).slice(-10).reverse() : []),
    [runs, draft?.id]
  )

  /** Открыть задачу в редакторе. `false` - человек не захотел бросать несохранённое. */
  const select = (t: TaskDef | null): boolean => {
    if (running) return false
    if (dirty && !confirm('Изменения задачи не сохранены. Бросить их?')) return false
    setDraft(t ? withStepIds(structuredClone(t)) : null)
    setDirty(false)
    setError('')
    setNote('')
    setProgress([])
    setReport(null)
    setRunProfile('')
    setAsking(null)
    return true
  }

  const patch = (p: Partial<TaskDef>): void => {
    setDraft((d) => (d ? { ...d, ...p } : d))
    setDirty(true)
  }

  const patchStep = (i: number, p: Partial<TaskStep>): void => {
    setDraft((d) => {
      if (!d) return d
      const steps = [...d.steps]
      steps[i] = { ...steps[i], ...p }
      return { ...d, steps }
    })
    setDirty(true)
  }

  const variables = draft?.variables ?? []
  const profiles = draft?.profiles ?? []

  const patchVar = (i: number, p: Partial<TaskVariable>): void => {
    const next = [...variables]
    next[i] = { ...next[i], ...p }
    patch({ variables: next })
  }

  const patchProfile = (i: number, p: Partial<TaskProfile>): void => {
    const next = [...profiles]
    next[i] = { ...next[i], ...p }
    patch({ profiles: next })
  }

  /** Пустое значение среды - «не задано», а не пустая строка: иначе оно перекрыло бы умолчание. */
  const setProfileValue = (i: number, name: string, value: string): void => {
    const values = { ...profiles[i].values }
    if (value === '') delete values[name]
    else values[name] = value
    patchProfile(i, { values })
  }

  const toggleServer = (id: string): void => {
    if (!draft) return
    const has = draft.serverIds.includes(id)
    patch({ serverIds: has ? draft.serverIds.filter((x) => x !== id) : [...draft.serverIds, id] })
  }

  const save = async (): Promise<TaskDef | null> => {
    if (!draft) return null
    try {
      const saved = await window.api.tasks.save(draft)
      const merged = { ...draft, id: saved.id }
      setDraft(merged)
      setDirty(false)
      await load()
      return merged
    } catch (e) {
      setError(errText(e))
      return null
    }
  }

  const remove = async (): Promise<void> => {
    if (!draft?.id || !confirm(`Удалить задачу «${draft.name}»? История её запусков останется.`)) return
    try {
      await window.api.tasks.remove(draft.id)
      setDraft(null)
      setDirty(false)
      await load()
    } catch (e) {
      setError(errText(e))
    }
  }

  const fromTemplate = (id: string): void => {
    const tpl = TEMPLATES.find((x) => x.id === id)
    if (tpl && select(tpl.make())) {
      setDirty(true)
      setNote(`Задача из шаблона «${tpl.label}». Отметьте серверы, проверьте значения и сохраните.`)
    }
  }

  const importTask = async (): Promise<void> => {
    if (running || (dirty && !confirm('Изменения задачи не сохранены. Бросить их?'))) return
    try {
      const r = await window.api.tasks.importFrom()
      if (!r.imported || !r.task) return
      await load()
      setDirty(false)
      select(r.task)
      setNote(
        r.missing && r.missing.length > 0
          ? `Задача загружена. На этой машине не нашлись серверы: ${r.missing.join(', ')} - отметьте нужные вручную.`
          : 'Задача загружена.'
      )
    } catch (e) {
      setError(errText(e))
    }
  }

  const exportTask = async (): Promise<void> => {
    const t = dirty || !draft?.id ? await save() : draft
    if (!t?.id) return
    try {
      const r = await window.api.tasks.exportTo(t.id, t.name)
      if (r.saved) setNote(`Задача выгружена: ${r.path}. Секретов в файле нет.`)
    } catch (e) {
      setError(errText(e))
    }
  }

  const execute = async (dry: boolean, values: Record<string, string>): Promise<void> => {
    if (!draft) return
    const task = dirty || !draft.id ? await save() : draft
    if (!task) return
    const runId = crypto.randomUUID()
    runIdRef.current = runId
    setError('')
    setNote('')
    setReport(null)
    setProgress([])
    setDryRun(dry)
    setRunning(true)
    setAsking(null)
    // Подписка - до запуска: первые события приходят раньше, чем отрисуется окно.
    const off = window.api.tasks.onProgress((ev) =>
      setProgress((prev) => applyProgress(prev, ev, runId, task.steps.length))
    )
    try {
      // Значения запуска идут только в сам запуск: в файл задачи они не попадают.
      const r = await window.api.tasks.run({ ...task, runProfile: runProfile || undefined, runValues: values }, runId, dry)
      setReport(r)
      setProgress(progressFromReport(r, task.steps.length))
      if (!dry) setRuns(await window.api.tasks.runs())
    } catch (e) {
      setError(errText(e))
    } finally {
      off()
      runIdRef.current = null
      setRunning(false)
    }
  }

  const start = (dry: boolean): void => {
    if (!draft) return
    const problem = taskProblem(draft)
    if (problem) {
      setError(problem)
      return
    }
    const targets = effectiveServers(draft, runProfile || undefined)
    if (targets.length === 0) {
      setError('Не выбран ни один сервер')
      return
    }
    if (
      !dry &&
      !confirm(
        `Запустить «${draft.name}»${runProfile ? ` в среде «${runProfile}»` : ''} на ${targets.length} серверах?\n\nПробный прогон покажет, что будет сделано, ничего не меняя.`
      )
    ) {
      return
    }
    if (promptVariables(draft).length > 0) {
      setAsking({ dry, values: initialValues(draft, runProfile || undefined) })
      return
    }
    void execute(dry, {})
  }

  const stop = (): void => {
    if (runIdRef.current) void window.api.tasks.cancel(runIdRef.current)
  }

  const showRun = (r: RunReport): void => {
    setReport(r)
    setDryRun(r.dryRun)
    setProgress(progressFromReport(r, r.servers[0]?.steps.length ?? draft?.steps.length ?? 0))
  }

  const labelOf = (serverIdx: number, stepIdx: number): string =>
    report?.servers[serverIdx]?.steps.find((s) => s.index === stepIdx)?.label ??
    (draft?.steps[stepIdx] ? stepLabel(draft.steps[stepIdx]) : `Шаг ${stepIdx + 1}`)

  const askVars = draft ? promptVariables(draft) : []
  const askReady = !!asking && askVars.every((v) => !v.secret || (asking.values[v.name] ?? '') !== '')

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && !running && onClose()}>
      <div className="modal tasks-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Задачи</h2>
        <div className="tasks-layout">
          <div className="tasks-list">
            <div className="tasks-list-tools">
              <button className="primary" disabled={running} onClick={() => select(emptyTask())}>
                Новая задача
              </button>
              <select value="" disabled={running} onChange={(e) => fromTemplate(e.target.value)} title="Готовые сценарии">
                <option value="">Из шаблона…</option>
                {TEMPLATES.map((t) => (
                  <option key={t.id} value={t.id} title={t.hint}>
                    {t.label}
                  </option>
                ))}
              </select>
              <button disabled={running} onClick={() => void importTask()}>
                Загрузить из файла…
              </button>
            </div>
            {tasks.length === 0 && <div className="hint">Задач пока нет.</div>}
            {tasks.map((t) => (
              <button
                key={t.id}
                className={'tasks-list-item' + (draft?.id === t.id ? ' on' : '')}
                disabled={running}
                onClick={() => select(t)}
              >
                <span className="tasks-list-name">{t.name}</span>
                <span className="tasks-list-meta">
                  шагов {t.steps.length} · серверов {t.serverIds.length}
                  {(t.profiles ?? []).length > 0 ? ` · сред ${(t.profiles ?? []).length}` : ''}
                </span>
              </button>
            ))}
          </div>

          <div className="tasks-editor">
            {!draft && <div className="hint">Выберите задачу слева, создайте новую или возьмите шаблон.</div>}
            {draft && (
              <>
                <div className="task-head">
                  <input
                    className="task-name"
                    value={draft.name}
                    disabled={running}
                    onChange={(e) => patch({ name: e.target.value })}
                  />
                  <button disabled={running || !dirty} onClick={() => void save()}>
                    Сохранить
                  </button>
                  <button disabled={running} onClick={() => void exportTask()} title="Файл для другой машины, без секретов">
                    Выгрузить…
                  </button>
                  {draft.id && (
                    <button className="danger" disabled={running} onClick={() => void remove()}>
                      Удалить
                    </button>
                  )}
                </div>
                {error && <div className="settings-msg err">{error}</div>}
                {note && <div className="settings-msg ok">{note}</div>}

                <fieldset className="task-section" disabled={running}>
                  <legend>Шаги - по порядку на каждом сервере</legend>
                  {draft.steps.length === 0 && <div className="hint">Шагов пока нет.</div>}
                  {draft.steps.map((step, i) => (
                    <div key={step.id} className="task-step">
                      <div className="task-step-head">
                        <span className="task-step-num">{i + 1}</span>
                        <span className="task-step-kind">{STEP_KIND_LABEL[step.kind]}</span>
                        <input
                          className="task-step-name"
                          value={step.name ?? ''}
                          placeholder={stepLabel({ ...step, name: '' })}
                          onChange={(e) => patchStep(i, { name: e.target.value })}
                        />
                        <button className="mini" title="Выше" onClick={() => patch({ steps: moveStep(draft.steps, i, -1) })}>
                          ↑
                        </button>
                        <button className="mini" title="Ниже" onClick={() => patch({ steps: moveStep(draft.steps, i, 1) })}>
                          ↓
                        </button>
                        <button
                          className="mini danger"
                          title="Убрать шаг"
                          onClick={() => patch({ steps: draft.steps.filter((_, k) => k !== i) })}
                        >
                          ✕
                        </button>
                      </div>
                      <div className="task-step-fields">
                        <StepFields step={step} onChange={(p) => patchStep(i, p)} />
                      </div>
                      <div className="task-step-common">
                        <label className="task-field">
                          Выполнять
                          <select
                            value={step.when ?? 'success'}
                            onChange={(e) => patchStep(i, { when: e.target.value as StepWhen })}
                          >
                            {(Object.keys(WHEN_LABEL) as StepWhen[]).map((w) => (
                              <option key={w} value={w}>
                                {WHEN_LABEL[w]}
                              </option>
                            ))}
                          </select>
                        </label>
                        <label className="task-field">
                          Повторов при ошибке
                          <input
                            type="number"
                            min={0}
                            max={10}
                            value={step.retries ?? 0}
                            onChange={(e) => patchStep(i, { retries: Math.min(10, Math.max(0, Number(e.target.value) || 0)) })}
                          />
                        </label>
                        <label className="task-field">
                          Срок шага, с
                          <input
                            type="number"
                            min={1}
                            placeholder="300"
                            value={step.timeoutSec ?? ''}
                            onChange={(e) => patchStep(i, { timeoutSec: e.target.value ? Math.max(1, Number(e.target.value)) : undefined })}
                          />
                        </label>
                        <label className="checkbox-row">
                          <input
                            type="checkbox"
                            checked={!!step.continueOnError}
                            onChange={(e) => patchStep(i, { continueOnError: e.target.checked })}
                          />
                          Продолжать при ошибке
                        </label>
                      </div>
                    </div>
                  ))}
                  <div className="task-add">
                    <select value={addKind} onChange={(e) => setAddKind(e.target.value as StepKind)}>
                      {STEP_KINDS.map((k) => (
                        <option key={k} value={k}>
                          {STEP_KIND_LABEL[k]}
                        </option>
                      ))}
                    </select>
                    <button onClick={() => patch({ steps: [...draft.steps, newStep(addKind)] })}>Добавить шаг</button>
                  </div>
                </fieldset>

                <fieldset className="task-section" disabled={running}>
                  <legend>Переменные</legend>
                  <div className="hint">
                    В полях шагов пишите {'{{имя}}'}. Встроенные, свои у каждого сервера: {'{{server.name}}'},{' '}
                    {'{{server.host}}'}, {'{{server.user}}'}. Секрет не сохраняется: его спросят перед запуском, а в
                    выводе он заменится на ••••.
                  </div>
                  {variables.map((v, i) => (
                    <div key={i} className="task-var">
                      <input
                        placeholder="имя"
                        value={v.name}
                        spellCheck={false}
                        onChange={(e) => patchVar(i, { name: e.target.value })}
                      />
                      {v.secret ? (
                        <input disabled placeholder="спросится при запуске" />
                      ) : (
                        <input
                          placeholder="значение по умолчанию"
                          value={v.default ?? ''}
                          spellCheck={false}
                          onChange={(e) => patchVar(i, { default: e.target.value })}
                        />
                      )}
                      <label className="checkbox-row">
                        <input
                          type="checkbox"
                          checked={!!v.ask || !!v.secret}
                          disabled={!!v.secret}
                          onChange={(e) => patchVar(i, { ask: e.target.checked })}
                        />
                        спрашивать
                      </label>
                      <label className="checkbox-row">
                        <input
                          type="checkbox"
                          checked={!!v.secret}
                          onChange={(e) => patchVar(i, { secret: e.target.checked, default: e.target.checked ? '' : v.default })}
                        />
                        секрет
                      </label>
                      <button
                        className="mini danger"
                        title="Убрать переменную"
                        onClick={() => patch({ variables: variables.filter((_, k) => k !== i) })}
                      >
                        ✕
                      </button>
                    </div>
                  ))}
                  <div className="task-add">
                    <button onClick={() => patch({ variables: [...variables, { name: '', default: '' }] })}>
                      Добавить переменную
                    </button>
                  </div>
                </fieldset>

                <fieldset className="task-section" disabled={running}>
                  <legend>Среды</legend>
                  <div className="hint">
                    Prod, stage, dev: у каждой свои значения переменных и, если нужно, свои серверы. Пустое поле - берётся
                    значение по умолчанию.
                  </div>
                  {profiles.map((p, i) => (
                    <div key={i} className="task-profile">
                      <div className="task-profile-head">
                        <input
                          placeholder="название среды"
                          value={p.name}
                          onChange={(e) => {
                            if (runProfile === p.name) setRunProfile(e.target.value)
                            patchProfile(i, { name: e.target.value })
                          }}
                        />
                        <span className="hint">
                          {p.serverIds.length > 0 ? `свои серверы: ${p.serverIds.length}` : 'серверы задачи'}
                        </span>
                        <button
                          className="mini"
                          disabled={draft.serverIds.length === 0}
                          title="Запомнить для этой среды серверы, отмеченные ниже"
                          onClick={() => patchProfile(i, { serverIds: [...draft.serverIds] })}
                        >
                          Взять отмеченные серверы
                        </button>
                        {p.serverIds.length > 0 && (
                          <button className="mini" onClick={() => patchProfile(i, { serverIds: [] })}>
                            Сбросить серверы
                          </button>
                        )}
                        <button
                          className="mini danger"
                          title="Убрать среду"
                          onClick={() => {
                            if (runProfile === p.name) setRunProfile('')
                            patch({ profiles: profiles.filter((_, k) => k !== i) })
                          }}
                        >
                          ✕
                        </button>
                      </div>
                      {variables.some((v) => !v.secret && v.name.trim()) && (
                        <div className="task-profile-values">
                          {variables
                            .filter((v) => !v.secret && v.name.trim())
                            .map((v) => (
                              <label key={v.name} className="task-field">
                                {v.name}
                                <input
                                  value={p.values[v.name] ?? ''}
                                  placeholder={v.default ?? ''}
                                  spellCheck={false}
                                  onChange={(e) => setProfileValue(i, v.name, e.target.value)}
                                />
                              </label>
                            ))}
                        </div>
                      )}
                    </div>
                  ))}
                  <div className="task-add">
                    <button onClick={() => patch({ profiles: [...profiles, { name: '', values: {}, serverIds: [] }] })}>
                      Добавить среду
                    </button>
                  </div>
                </fieldset>

                <fieldset className="task-section" disabled={running}>
                  <legend>
                    Серверы - выбрано {draft.serverIds.length}, по{' '}
                    <select
                      value={draft.concurrency ?? 4}
                      onChange={(e) => patch({ concurrency: Number(e.target.value) })}
                    >
                      {[1, 2, 4, 8, 16, 32].map((n) => (
                        <option key={n} value={n}>
                          {n}
                        </option>
                      ))}
                    </select>{' '}
                    одновременно
                  </legend>
                  <input
                    className="task-server-filter"
                    placeholder="Найти сервер"
                    value={serverFilter}
                    onChange={(e) => setServerFilter(e.target.value)}
                  />
                  <div className="task-servers">
                    {choices.map((s) => (
                      <label key={s.id} className="checkbox-row">
                        <input type="checkbox" checked={draft.serverIds.includes(s.id)} onChange={() => toggleServer(s.id)} />
                        {s.name}
                        <span className="multi-host">
                          {s.username}@{s.host}
                        </span>
                      </label>
                    ))}
                  </div>
                </fieldset>

                {asking && (
                  <div className="task-ask">
                    <div className="settings-section-title">
                      {asking.dry ? 'Значения для пробного прогона' : 'Значения для запуска'}
                    </div>
                    {askVars.map((v) => (
                      <label key={v.name} className="task-field wide">
                        {v.name}
                        {v.secret ? ' (секрет - не сохраняется)' : ''}
                        <input
                          type={v.secret ? 'password' : 'text'}
                          autoFocus={v === askVars[0]}
                          spellCheck={false}
                          value={asking.values[v.name] ?? ''}
                          onChange={(e) => setAsking({ ...asking, values: { ...asking.values, [v.name]: e.target.value } })}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter' && askReady) void execute(asking.dry, asking.values)
                          }}
                        />
                      </label>
                    ))}
                    <div className="task-ask-actions">
                      <button className="primary" disabled={!askReady} onClick={() => void execute(asking.dry, asking.values)}>
                        {asking.dry ? 'Пробный прогон' : 'Запустить'}
                      </button>
                      <button onClick={() => setAsking(null)}>Отмена</button>
                      {!askReady && <span className="hint">Секретные значения нужно ввести.</span>}
                    </div>
                  </div>
                )}

                <div className="task-run">
                  {profiles.length > 0 && (
                    <select value={runProfile} disabled={running} onChange={(e) => setRunProfile(e.target.value)} title="Среда запуска">
                      <option value="">Без среды</option>
                      {profiles
                        .filter((p) => p.name.trim())
                        .map((p) => (
                          <option key={p.name} value={p.name}>
                            Среда: {p.name}
                          </option>
                        ))}
                    </select>
                  )}
                  <button disabled={running || !!asking} onClick={() => start(true)}>
                    Пробный прогон
                  </button>
                  <button className="primary" disabled={running || !!asking} onClick={() => start(false)}>
                    Запустить
                  </button>
                  {running && (
                    <button className="danger" onClick={stop}>
                      Остановить
                    </button>
                  )}
                  {(running || progress.length > 0) && (
                    <span className="task-run-summary">
                      {dryRun ? 'Пробный прогон: ' : ''}
                      {runSummary(progress)}
                    </span>
                  )}
                </div>

                {progress.length > 0 && (
                  <div className="task-progress">
                    {progress.map((srv, si) => (
                      <details key={srv.serverId} className={'task-progress-row ' + srv.state} open={progress.length === 1}>
                        <summary>
                          <span className="task-progress-name">{srv.name}</span>
                          <span className="task-progress-state">{SERVER_STATE_LABEL[srv.state] ?? srv.state}</span>
                          <span className="task-chips">
                            {srv.steps.map((st, k) => (
                              <span key={k} className={'task-chip ' + st.state} title={`${k + 1}. ${STEP_STATE_LABEL[st.state]}`} />
                            ))}
                          </span>
                        </summary>
                        {srv.error && <pre className="task-output">{srv.error}</pre>}
                        {srv.steps.map((st, k) => (
                          <div key={k} className={'task-progress-step ' + st.state}>
                            <div>
                              {k + 1}. {labelOf(si, k)} - <b>{STEP_STATE_LABEL[st.state]}</b>
                            </div>
                            {st.output && <pre className="task-output">{st.output}</pre>}
                          </div>
                        ))}
                      </details>
                    ))}
                  </div>
                )}

                {history.length > 0 && (
                  <div className="task-history">
                    <div className="settings-section-title">История запусков</div>
                    {history.map((r) => (
                      <button key={r.runId} className="mini task-history-item" disabled={running} onClick={() => showRun(r)}>
                        {new Date(r.startedAt).toLocaleString('ru-RU')} · {runSummary(r.servers)}
                        {r.cancelled ? ' · остановлен' : ''}
                      </button>
                    ))}
                  </div>
                )}
              </>
            )}
          </div>
        </div>

        <div className="modal-actions">
          <button disabled={running} onClick={() => (dirty && !confirm('Изменения задачи не сохранены. Закрыть?') ? undefined : onClose())}>
            Закрыть
          </button>
        </div>
      </div>
    </div>
  )
}
