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

/** Переменная задачи: `{{имя}}` в полях шагов. */
export interface TaskVariable {
  name: string
  default?: string
  /** Спросить перед запуском, подставив умолчание. */
  ask?: boolean
  /** Не хранится: вводится перед каждым запуском, в выводе заменяется на ••••. */
  secret?: boolean
  description?: string
}

/** Среда: свои значения переменных и, если заданы, свои серверы. */
export interface TaskProfile {
  name: string
  values: Record<string, string>
  serverIds: string[]
}

export interface TaskDef {
  id?: string
  name: string
  steps: TaskStep[]
  serverIds: string[]
  concurrency?: number
  variables?: TaskVariable[]
  profiles?: TaskProfile[]
  /** Только на время запуска: выбранная среда и введённые значения. Не сохраняются. */
  runProfile?: string
  runValues?: Record<string, string>
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
  if (t.serverIds.length === 0 && !(t.profiles ?? []).some((p) => p.serverIds.length > 0)) {
    return 'Не выбран ни один сервер'
  }
  const varProblem = variableProblem(t)
  if (varProblem) return varProblem
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
        if (s.check === 'port' && references(s.target ?? '').length === 0 && !/^.+:\d{1,5}$/.test((s.target ?? '').trim())) {
          return n + 'порт указывается как адрес:порт'
        }
        if (s.check === 'http' && references(s.target ?? '').length === 0 && !/^https?:\/\//.test((s.target ?? '').trim())) {
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

// ---- Переменные, среды, шаблоны ----

/** Встроенные переменные - свои у каждого сервера. */
export const BUILTIN_VARS = ['server.name', 'server.host', 'server.user']

const NAME_RE = /^[A-Za-z_][A-Za-z0-9_.-]*$/

/** Имена переменных в тексте - по тем же правилам, что у бэкенда: `{{.Names}}` не переменная. */
export function references(text: string): string[] {
  const out: string[] = []
  let rest = text
  for (;;) {
    const start = rest.indexOf('{{')
    if (start < 0) break
    const after = rest.slice(start + 2)
    const end = after.indexOf('}}')
    if (end < 0) break
    const name = after.slice(0, end).trim()
    if (NAME_RE.test(name)) out.push(name)
    rest = after.slice(end + 2)
  }
  return out
}

function stepTexts(s: TaskStep): string[] {
  return [s.command, s.localPath, s.remotePath, s.service, s.action, s.container, s.target].filter(
    (x): x is string => typeof x === 'string'
  )
}

/** Ошибка в переменных и средах или `null`. */
export function variableProblem(t: TaskDef): string | null {
  const names = new Set<string>()
  for (const v of t.variables ?? []) {
    const n = v.name.trim()
    if (!NAME_RE.test(n)) return `Переменная «${v.name}»: имя из букв, цифр, «_», «.», «-», начиная с буквы`
    if (n.startsWith('server.')) return `Переменная «${n}»: имена server.* заняты встроенными`
    if (names.has(n)) return `Переменная «${n}» объявлена дважды`
    names.add(n)
  }
  const profiles = new Set<string>()
  for (const p of t.profiles ?? []) {
    const n = p.name.trim()
    if (!n) return 'У среды нет названия'
    if (profiles.has(n)) return `Среда «${n}» объявлена дважды`
    profiles.add(n)
  }
  for (let i = 0; i < t.steps.length; i++) {
    for (const text of stepTexts(t.steps[i])) {
      for (const r of references(text)) {
        if (!names.has(r) && !BUILTIN_VARS.includes(r)) return `Шаг ${i + 1}: неизвестная переменная «${r}»`
      }
    }
  }
  return null
}

/** Серверы запуска: у выбранной среды свои, если заданы. */
export function effectiveServers(t: TaskDef, profile?: string): string[] {
  const p = profile ? (t.profiles ?? []).find((x) => x.name === profile) : undefined
  return p && p.serverIds.length > 0 ? p.serverIds : t.serverIds
}

/** Что спросить перед запуском: помеченные «спрашивать» и все секретные. */
export function promptVariables(t: TaskDef): TaskVariable[] {
  return (t.variables ?? []).filter((v) => v.ask || v.secret)
}

/** Начальные значения окна запуска: умолчание, затем среда. Секреты всегда пустые. */
export function initialValues(t: TaskDef, profile?: string): Record<string, string> {
  const p = profile ? (t.profiles ?? []).find((x) => x.name === profile) : undefined
  const out: Record<string, string> = {}
  for (const v of t.variables ?? []) out[v.name] = v.secret ? '' : (p?.values[v.name] ?? v.default ?? '')
  return out
}

export interface TaskTemplate {
  id: string
  label: string
  hint: string
  make: () => TaskDef
}

const tplStep = (kind: StepKind, p: Partial<TaskStep>): TaskStep => ({ ...newStep(kind), ...p })

/** Готовые задачи: частые сценарии, в которых остаётся подставить свои значения. */
export const TEMPLATES: TaskTemplate[] = [
  {
    id: 'service-restart',
    label: 'Перезапуск службы с проверкой',
    hint: 'restart службы, проверка адреса, при ошибке - последние строки её журнала',
    make: () => ({
      name: 'Перезапуск службы',
      serverIds: [],
      concurrency: 1,
      variables: [
        { name: 'service', default: 'nginx', ask: true },
        { name: 'url', default: 'http://127.0.0.1/' }
      ],
      steps: [
        tplStep('service', { service: '{{service}}', action: 'restart' }),
        tplStep('healthcheck', { check: 'http', target: '{{url}}', attempts: 5, intervalSec: 3 }),
        tplStep('command', {
          name: 'Журнал службы при ошибке',
          command: 'journalctl -u {{service}} -n 50 --no-pager',
          when: 'failure'
        })
      ]
    })
  },
  {
    id: 'deploy',
    label: 'Выкладка: залить, перезапустить, проверить',
    hint: 'синхронизация своей папки на сервер, restart службы, проверка адреса',
    make: () => ({
      name: 'Выкладка',
      serverIds: [],
      concurrency: 1,
      variables: [
        { name: 'localDir', default: '', ask: true },
        { name: 'remoteDir', default: '/var/www/site' },
        { name: 'service', default: 'nginx' },
        { name: 'url', default: 'http://127.0.0.1/' }
      ],
      steps: [
        tplStep('sync', { localPath: '{{localDir}}', remotePath: '{{remoteDir}}' }),
        tplStep('service', { service: '{{service}}', action: 'restart' }),
        tplStep('healthcheck', { check: 'http', target: '{{url}}', attempts: 5, intervalSec: 3 })
      ]
    })
  },
  {
    id: 'compose-update',
    label: 'Обновить контейнеры Docker Compose',
    hint: 'pull и up -d в каталоге проекта, затем проверка, что ни один контейнер не остановился',
    make: () => ({
      name: 'Обновление Compose',
      serverIds: [],
      concurrency: 1,
      variables: [{ name: 'dir', default: '/opt/app', ask: true }],
      steps: [
        tplStep('command', { command: 'cd {{dir}} && docker compose pull && docker compose up -d' }),
        tplStep('healthcheck', {
          check: 'command',
          target: 'cd {{dir}} && test -z "$(docker compose ps --status exited -q)"',
          attempts: 3,
          intervalSec: 5
        })
      ]
    })
  }
]
