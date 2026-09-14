/**
 * Задачи: упорядоченные шаги на выбранных серверах.
 *
 * Здесь - всё, что окну нужно знать о задаче без сети: какие шаги бывают, чем заполнить
 * новый, чего не хватает до запуска и как из событий бэкенда собрать ход прогона. Выполняет
 * задачи бэкенд (`tasks.rs`), и проверяет он же - здесь проверка только раньше подсказывает.
 */

export type StepKind = 'command' | 'upload' | 'download' | 'sync' | 'service' | 'docker' | 'healthcheck'
export type StepWhen = 'success' | 'failure' | 'always'
export type CheckKind = 'command' | 'port' | 'http'

export interface TaskStep {
  /** Только для окна: ключ строки в списке. */
  id: string
  kind: StepKind
  name?: string
  when?: StepWhen
  continueOnError?: boolean
  retries?: number
  timeoutSec?: number
  command?: string
  localPath?: string
  remotePath?: string
  includeRemoteNewer?: boolean
  service?: string
  action?: string
  container?: string
  check?: CheckKind
  target?: string
  attempts?: number
  intervalSec?: number
}

export interface TaskDef {
  id?: string
  name: string
  steps: TaskStep[]
  serverIds: string[]
  concurrency?: number
}

export const STEP_KINDS: StepKind[] = ['command', 'upload', 'download', 'sync', 'service', 'docker', 'healthcheck']

export const STEP_KIND_LABEL: Record<StepKind, string> = {
  command: 'Команда',
  upload: 'Загрузка на сервер',
  download: 'Скачивание с сервера',
  sync: 'Синхронизация папки',
  service: 'Служба',
  docker: 'Контейнер Docker',
  healthcheck: 'Проверка работоспособности'
}

export const WHEN_LABEL: Record<StepWhen, string> = {
  success: 'если до этого всё прошло',
  failure: 'только если задача остановилась на ошибке',
  always: 'в любом случае'
}

export const CHECK_LABEL: Record<CheckKind, string> = {
  command: 'команда завершается с кодом 0',
  port: 'порт принимает соединения',
  http: 'адрес отвечает по HTTP'
}

export const SERVICE_ACTIONS = ['start', 'stop', 'restart']
export const DOCKER_ACTIONS = ['start', 'stop', 'restart', 'remove']

function uid(): string {
  return typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : Math.random().toString(36).slice(2)
}

export function newStep(kind: StepKind): TaskStep {
  const base: TaskStep = { id: uid(), kind, when: 'success', continueOnError: false, retries: 0 }
  switch (kind) {
    case 'command':
      return { ...base, command: '' }
    case 'upload':
    case 'sync':
      return { ...base, localPath: '', remotePath: '' }
    case 'download':
      return { ...base, remotePath: '', localPath: '' }
    case 'service':
      return { ...base, service: '', action: 'restart' }
    case 'docker':
      return { ...base, container: '', action: 'restart' }
    case 'healthcheck':
      return { ...base, check: 'http', target: '', attempts: 3, intervalSec: 5 }
  }
}

function firstLine(s: string): string {
  const line = (s.split('\n')[0] ?? '').trim()
  return line.length > 60 ? `${line.slice(0, 60)}…` : line
}

/** Название шага словами - так же, как его назовёт бэкенд в отчёте. */
export function stepLabel(s: TaskStep): string {
  if (s.name?.trim()) return s.name.trim()
  switch (s.kind) {
    case 'command':
      return `Команда: ${firstLine(s.command ?? '')}`
    case 'upload':
      return `Залить ${s.localPath ?? ''} → ${s.remotePath ?? ''}`
    case 'download':
      return `Скачать ${s.remotePath ?? ''} → ${s.localPath ?? ''}`
    case 'sync':
      return `Синхронизировать ${s.localPath ?? ''} → ${s.remotePath ?? ''}`
    case 'service':
      return `Служба ${s.service ?? ''}: ${s.action ?? ''}`
    case 'docker':
      return `Контейнер ${s.container ?? ''}: ${s.action ?? ''}`
    case 'healthcheck': {
      const what = s.check === 'port' ? 'порта' : s.check === 'command' ? 'командой' : 'адреса'
      return `Проверка ${what}: ${s.target ?? ''}`
    }
  }
}

const blank = (v?: string): boolean => !v || !v.trim()

/** Чего не хватает до запуска, или `null`. Номер шага - в тексте: так его и ищут. */
export function taskProblem(t: TaskDef): string | null {
  if (blank(t.name)) return 'У задачи нет названия'
  if (t.steps.length === 0) return 'В задаче нет шагов'
  if (t.serverIds.length === 0) return 'Не выбран ни один сервер'
  for (let i = 0; i < t.steps.length; i++) {
    const s = t.steps[i]
    const n = `Шаг ${i + 1}: `
    switch (s.kind) {
      case 'command':
        if (blank(s.command)) return n + 'пустая команда'
        break
      case 'upload':
      case 'download':
      case 'sync':
        if (blank(s.localPath) || blank(s.remotePath)) return n + 'нужны и своя папка, и путь на сервере'
        if ((s.remotePath ?? '').split('/').includes('..')) return n + 'в пути на сервере не бывает «..»'
        break
      case 'service':
        if (blank(s.service)) return n + 'не указана служба'
        break
      case 'docker':
        if (blank(s.container)) return n + 'не указан контейнер'
        break
      case 'healthcheck':
        if (blank(s.target)) return n + 'не указано, что проверять'
        if (s.check === 'port' && !/^.+:\d{1,5}$/.test((s.target ?? '').trim())) {
          return n + 'порт указывается как адрес:порт'
        }
        if (s.check === 'http' && !/^https?:\/\//.test((s.target ?? '').trim())) {
          return n + 'адрес начинается с http:// или https://'
        }
        break
    }
  }
  return null
}

export function moveStep(steps: TaskStep[], index: number, dir: -1 | 1): TaskStep[] {
  const to = index + dir
  if (index < 0 || index >= steps.length || to < 0 || to >= steps.length) return steps
  const next = [...steps]
  ;[next[index], next[to]] = [next[to], next[index]]
  return next
}

// ---- Ход прогона ----

export type StepState = 'pending' | 'running' | 'done' | 'failed' | 'skipped' | 'cancelled' | 'planned' | 'problem'

export const STEP_STATE_LABEL: Record<StepState, string> = {
  pending: 'ждёт',
  running: 'выполняется',
  done: 'готово',
  failed: 'ошибка',
  skipped: 'пропущен',
  cancelled: 'остановлен',
  planned: 'будет выполнен',
  problem: 'не выполнится'
}

export interface ServerProgress {
  serverId: string
  name: string
  /** connecting / running / done / failed / skipped / cancelled / problem */
  state: string
  error?: string
  steps: { state: StepState; output?: string }[]
}

export interface ProgressEvent {
  runId: string
  serverId: string
  name: string
  step: number | null
  state: string
  output?: string | null
}

/** Событие бэкенда поверх хода прогона. Чужой прогон не трогает. */
export function applyProgress(
  servers: ServerProgress[],
  ev: ProgressEvent,
  runId: string,
  stepCount: number
): ServerProgress[] {
  if (ev.runId !== runId) return servers
  const idx = servers.findIndex((s) => s.serverId === ev.serverId)
  const cur: ServerProgress =
    idx >= 0
      ? servers[idx]
      : {
          serverId: ev.serverId,
          name: ev.name,
          state: 'connecting',
          steps: Array.from({ length: stepCount }, () => ({ state: 'pending' as StepState }))
        }
  let next: ServerProgress
  if (ev.step === null || ev.step === undefined) {
    next = { ...cur, state: ev.state, error: ev.output ?? cur.error }
  } else {
    const steps = [...cur.steps]
    steps[ev.step] = { state: ev.state as StepState, output: ev.output ?? steps[ev.step]?.output }
    next = { ...cur, state: cur.state === 'connecting' ? 'running' : cur.state, steps }
  }
  if (idx < 0) return [...servers, next]
  const out = [...servers]
  out[idx] = next
  return out
}

/** Итог прогона словами: «готово 3 · ошибка 1 · пропущено 1». */
export function runSummary(servers: { state: string }[]): string {
  const labels: Record<string, string> = {
    done: 'готово',
    failed: 'ошибка',
    skipped: 'пропущено',
    cancelled: 'остановлено',
    problem: 'с проблемами',
    connecting: 'подключается',
    running: 'выполняется'
  }
  const counts = new Map<string, number>()
  for (const s of servers) counts.set(s.state, (counts.get(s.state) ?? 0) + 1)
  const order = ['done', 'problem', 'failed', 'skipped', 'cancelled', 'running', 'connecting']
  return (
    order
      .filter((k) => counts.has(k))
      .map((k) => `${labels[k]} ${counts.get(k)}`)
      .join(' · ') || 'нет серверов'
  )
}

/** Отчёт прогона - как его вернул бэкенд и как он лежит в истории. */
export interface RunReport {
  runId: string
  taskId?: string | null
  taskName: string
  dryRun: boolean
  cancelled: boolean
  startedAt: number
  finishedAt: number
  servers: {
    serverId: string
    name: string
    state: string
    error?: string | null
    errors?: number
    ms: number
    steps: { index: number; label: string; state: StepState; output: string; ms: number; attempts: number }[]
  }[]
}

/** Ход прогона из готового отчёта: для итога и для записи из истории. */
export function progressFromReport(r: RunReport, stepCount: number): ServerProgress[] {
  return r.servers.map((s) => {
    const steps: { state: StepState; output?: string }[] = Array.from({ length: stepCount }, () => ({
      state: 'pending' as StepState
    }))
    for (const st of s.steps) {
      if (st.index >= 0 && st.index < stepCount) steps[st.index] = { state: st.state, output: st.output }
    }
    return { serverId: s.serverId, name: s.name, state: s.state, error: s.error ?? undefined, steps }
  })
}

/** У сохранённых шагов может не оказаться ключа строки - окну он нужен. */
export function withStepIds(t: TaskDef): TaskDef {
  return { ...t, steps: t.steps.map((s) => (s.id ? s : { ...s, id: newStep(s.kind).id })) }
}
