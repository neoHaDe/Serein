// Общие типы, используются и в main, и в renderer

export type AuthType = 'password' | 'key' | 'agent'

export type TunnelType = 'local' | 'remote' | 'dynamic'

export interface TunnelConfig {
  id: string
  type: TunnelType
  /** Локальный порт: local/dynamic — слушаем здесь; remote — форвардим сюда. */
  localPort: number
  /** Удалённый хост (только для local). */
  remoteHost?: string
  /** local — порт цели; remote — порт на сервере, который слушает. */
  remotePort?: number
  label?: string
}

export interface TunnelStatus {
  sessionId: string
  tunnelId: string
  active: boolean
  error?: string
}

export interface Snippet {
  id: string
  name: string
  command: string
}

/** Один промпт keyboard-interactive (2FA/OTP). */
export interface KIPrompt {
  prompt: string
  /** true — показывать ввод; false — скрывать (пароль/OTP). */
  echo: boolean
}

export type KeyType = 'ed25519' | 'rsa'

export interface GenerateKeyParams {
  type: KeyType
  /** Только для RSA: длина ключа (2048/4096). */
  bits?: number
  comment?: string
  /** Парольная фраза для шифрования приватного ключа. */
  passphrase?: string
}

export interface GeneratedKey {
  /** Приватный ключ (OpenSSH PEM). */
  privateKey: string
  /** Публичный ключ (формат authorized_keys). */
  publicKey: string
}

export interface ServerConfig {
  id: string
  name: string
  host: string
  port: number
  username: string
  authType: AuthType
  /** Пароль (для authType==='password'). В хранилище шифруется. */
  password?: string
  /** Путь к приватному ключу (для authType==='key'). */
  privateKeyPath?: string
  /** Парольная фраза ключа. В хранилище шифруется. */
  passphrase?: string
  /** Группа/папка для отображения в сайдбаре. */
  group?: string
  /** Цвет ярлыка. */
  color?: string
  /** ID другого сохранённого сервера, через который подключаться (ProxyJump/бастион).
   *  Цепочка резолвится рекурсивно: целевой → jump1 → jump2 → … */
  proxyJump?: string
  /** Туннели, которые автоматически поднимаются после подключения. */
  tunnels?: TunnelConfig[]
  /** Shell-команда, отправляемая сразу после открытия сессии. */
  executeOnConnect?: string
  /** Пробрасывать SSH-агент на сервер (agent forwarding). */
  agentForward?: boolean
  /**
   * Отпечаток ключа из агента (`SHA256:…`), которым подключаться.
   * Пусто — перебирать все ключи агента подряд.
   */
  agentKey?: string
}

/** Ключ, загруженный в локальный SSH-агент. */
export interface AgentIdentity {
  algo: string
  comment: string
  fingerprint: string
}

export interface AgentIdentitiesResult {
  ok: boolean
  keys?: AgentIdentity[]
  error?: string
}

export type SessionKind = 'ssh' | 'local'

export interface OpenSshPayload {
  serverId: string
  cols: number
  rows: number
}

export interface OpenLocalPayload {
  cols: number
  rows: number
  cwd?: string
}

export interface SessionData {
  id: string
  data: string
}

export interface SessionExit {
  id: string
  code?: number | null
  signal?: string | null
  error?: string
  /** user — сами закрыли вкладку/панель; drop — обрыв канала. */
  reason?: 'user' | 'drop'
}

export interface SessionStatus {
  id: string
  status: 'connecting' | 'connected' | 'closed' | 'error' | 'reconnecting'
  message?: string
}

export interface ResizePayload {
  id: string
  cols: number
  rows: number
}

// ---- SFTP ----

export interface SftpEntry {
  name: string
  /** 'file' | 'dir' | 'link' */
  type: 'file' | 'dir' | 'link' | 'other'
  size: number
  /** unix mtime в миллисекундах */
  mtime: number
  mode: number
  /** Цель symlink, если type==='link'. */
  target?: string | null
  /** Куда ведёт ссылка после follow. */
  linkType?: 'file' | 'dir' | 'broken' | null
}

export interface SftpListResult {
  path: string
  entries: SftpEntry[]
}

export interface TransferProgress {
  transferId: string
  transferred: number
  total: number
  filename: string
  direction: 'upload' | 'download'
  done: boolean
  error?: string
}

export type TransferDirection = 'upload' | 'download'
export type TransferState = 'queued' | 'active' | 'paused' | 'done' | 'error' | 'canceled'

/** Один элемент очереди передач (файл). Папки разворачиваются в набор файлов. */
export interface TransferItem {
  id: string
  sessionId: string
  direction: TransferDirection
  localPath: string
  remotePath: string
  /** Отображаемое имя (относительный путь внутри передаваемой папки). */
  filename: string
  size: number
  transferred: number
  state: TransferState
  error?: string
  /** Считается на фронте по дельтам transferred. */
  speedBps?: number
}

/** Запись локальной файловой системы (для двухпанельного менеджера). */
export interface LocalEntry {
  name: string
  type: 'file' | 'dir' | 'other'
  size: number
  mtime: number
}

export interface LocalListResult {
  path: string
  entries: LocalEntry[]
}

/** Статус сессии редактирования удалённого файла во внешнем редакторе. */
export interface RemoteEditStatus {
  sessionId: string
  remotePath: string
  state: 'opened' | 'uploading' | 'synced' | 'error' | 'stopped'
  error?: string
}

/** Контейнер Docker (для панели управления). */
export interface DockerContainer {
  id: string
  name: string
  image: string
  state: string
  status: string
  ports?: string
  created?: string
}
export interface DockerContainerStats {
  cpuPct: string
  memUsage: string
  memPct: string
  netIo?: string
  blockIo?: string
}
export interface DockerStatsResult {
  ok: boolean
  stats?: DockerContainerStats
  error?: string
}
export interface DockerListResult {
  ok: boolean
  containers?: DockerContainer[]
  error?: string
}
export type DockerAction = 'start' | 'stop' | 'restart' | 'remove'

export interface DockerLogsChunk {
  sessionId: string
  containerId: string
  chunk: string
}

export interface DockerContainerFileEntry {
  name: string
  kind: 'dir' | 'file' | 'link'
}

export interface DockerContainerFilesResult {
  ok: boolean
  path?: string
  entries?: DockerContainerFileEntry[]
  error?: string
}

export interface DockerComposeProject {
  name: string
  project: string
  status: string
  composeFile: string
}

export interface DockerComposeListResult {
  ok: boolean
  projects?: DockerComposeProject[]
  error?: string
}

export interface DockerComposeService {
  name: string
  service: string
  id: string
  image: string
  state: string
  status: string
  ports?: string
}

export interface DockerComposePsResult {
  ok: boolean
  services?: DockerComposeService[]
  error?: string
}

export type DockerComposeAction = 'up' | 'down' | 'start' | 'stop' | 'restart'

/** Снимок ресурсов удалённого сервера (для виджета мониторинга). */
export interface ServerMetrics {
  ok: boolean
  cores: number
  cpuPct: number
  load: [number, number, number]
  memTotalKb: number
  memUsedKb: number
  diskPct: number
  uptimeSec: number
  error?: string
}

/** Содержимое удалённого файла для встроенного редактора. */
export interface RemoteFileContent {
  content: string
  /** Тип переводов строк исходного файла — чтобы сохранить как было. */
  eol: 'lf' | 'crlf'
  /** POSIX-режим файла (для сохранения прав при записи). */
  mode: number
  /** mtime сервера в мс — база для детекта внешних изменений. */
  mtime: number
  /** Файл слишком большой для редактора. */
  tooLarge?: boolean
  /** Похоже на бинарный файл (есть \0). */
  binary?: boolean
}

export interface SftpPreview {
  kind: 'bytes' | 'tooLarge'
  size: number
  base64?: string
}

/** Результат сохранения файла встроенным редактором. */
export interface WriteFileResult {
  ok: boolean
  /** Новый mtime после записи. */
  mtime?: number
  /** Файл изменился на сервере с момента открытия (конфликт). */
  conflict?: boolean
  error?: string
}

export interface ApiResult<T = void> {
  ok: boolean
  data?: T
  error?: string
}

// ---- Настройки приложения ----

export interface AppSettings {
  /** Имя цветовой схемы (см. THEMES в renderer). */
  theme: string
  fontSize: number
  fontFamily: string
  /** Открывать локальный терминал при запуске. */
  openLocalOnStart: boolean
  /** Авто-переподключение SSH при обрыве. */
  autoReconnect: boolean
  /** Сохранённые ширины боковых панелей. */
  sidebarWidth?: number
  sftpWidth?: number
  /** Переопределения горячих клавиш: actionId → комбинация (пусто = дефолт). */
  keybindings?: Record<string, string>
  /** Восстанавливать открытые вкладки при запуске. */
  restoreTabsOnStart?: boolean
  /** Shell для локального терминала (Windows): 'auto' | 'pwsh' | 'powershell' | 'cmd' | 'wsl' | свой путь. */
  localShell?: string
  /** Плотность интерфейса: 'comfortable' (по умолчанию) | 'compact'. */
  density?: 'comfortable' | 'compact'
  /** Отдельные кнопки на панели задач для откреплённых окон. По умолчанию одно приложение. */
  auxInTaskbar?: boolean
  /** Параллельных SFTP-файлов (1–8, по умолчанию 4). */
  sftpConcurrency?: number
  /** Показывать скрытые файлы (имя начинается с точки) в SFTP. */
  sftpShowHidden?: boolean
  /** Сохранять расположение доп. панелей (SFTP / откреплённые окна) после перезапуска. */
  restoreAuxOnStart?: boolean
  /** Какие колонки SFTP-проводника показывать. Имя всегда включено. */
  sftpColOn?: Partial<Record<'name' | 'ext' | 'mode' | 'size' | 'mtime', boolean>>
  /** Ширины колонок SFTP-проводника в пикселях. */
  sftpColWidths?: Partial<Record<'name' | 'ext' | 'mode' | 'size' | 'mtime', number>>
  /** Колонка сортировки списка. */
  sftpSortCol?: 'name' | 'ext' | 'mode' | 'size' | 'mtime'
  /** Направление сортировки. */
  sftpSortDir?: 'asc' | 'desc'
}

// ---- Сохранение раскладки вкладок (для восстановления при запуске) ----

export interface SerializedLeaf {
  t: 'leaf'
  kind: 'ssh' | 'local'
  serverId?: string
  title: string
}
export interface SerializedSplit {
  t: 'split'
  dir: 'row' | 'col'
  sizes: [number, number]
  children: [SerializedPane, SerializedPane]
}
export type SerializedPane = SerializedLeaf | SerializedSplit
/** Инструмент в рельсе workspace SSH-вкладки. */
export const WORKSPACE_TOOLS = [
  'terminal',
  'docker',
  'logs',
  'processes',
  'services',
  'tunnels'
] as const
export type WorkspaceTool = (typeof WORKSPACE_TOOLS)[number]

export function parseWorkspaceTool(v: unknown, _sftpOpen?: boolean): WorkspaceTool {
  if (typeof v === 'string') {
    if (v === 'resources') return 'processes'
    if (v === 'files') return 'terminal'
    if ((WORKSPACE_TOOLS as readonly string[]).includes(v)) {
      return v as WorkspaceTool
    }
  }
  return 'terminal'
}

export interface WorkspaceProcess {
  pid: number
  user: string
  cpu: number
  mem: number
  stat: string
  cmd: string
}

export interface WorkspaceService {
  name: string
  unit: string
  load: string
  active: string
  sub: string
  desc: string
}

export interface SerializedTab {
  title: string
  root: SerializedPane
  sftpOpen?: boolean
  workspace?: WorkspaceTool
}

/** Откреплённое доп. окно (SFTP / логи Docker). Координаты — физические inner. */
export interface SavedAuxWindow {
  kind: 'sftp' | 'dockerLogs'
  serverId: string
  containerId?: string
  name?: string
  x: number
  y: number
  w: number
  h: number
}
export interface AuxLayout {
  windows: SavedAuxWindow[]
}

export const DEFAULT_SETTINGS: AppSettings = {
  theme: 'Tokyo Night',
  fontSize: 14,
  fontFamily: 'Cascadia Code, Consolas, "Courier New", monospace',
  openLocalOnStart: false,
  autoReconnect: false,
  sidebarWidth: 270,
  sftpWidth: 380,
  keybindings: {},
  restoreTabsOnStart: false,
  localShell: 'auto',
  density: 'comfortable',
  auxInTaskbar: false,
  sftpConcurrency: 4,
  restoreAuxOnStart: false,
  sftpShowHidden: false,
  sftpColOn: { name: true, ext: true, mode: true, size: true, mtime: true },
  sftpColWidths: { name: 200, ext: 64, mode: 52, size: 84, mtime: 136 },
  sftpSortCol: 'name',
  sftpSortDir: 'asc'
}
