import type {
  PaneKind,
  SavedAuxWindow,
  SerializedPane,
  SerializedTab,
  ServerConfig,
  WorkspaceTool
} from '../shared/types'
import { paneKindOf, parseWorkspaceTool } from '../shared/types'
import type { ReattachSftpPayload, ReattachWorkspacePayload } from './reattach'
import type { PaneLeaf, PaneNode } from './paneTree'
import {
  allLeaves,
  deserializePane,
  findLeaf,
  firstLeaf,
  makeLeaf,
  removeLeaf,
  serializePane,
  splitLeaf
} from './paneTree'

/**
 * Вкладка и правила её восстановления после перезапуска.
 *
 * Жило внутри `App.tsx` — и тип, и вся ветвящаяся логика запуска. Логика тут не
 * очевидная: что именно открывать зависит от двух независимых настроек, а во втором
 * случае вкладки достраиваются под сохранённые дополнительные окна. Проверить это, пока
 * оно сидело в `useEffect` посреди компонента, было нельзя вовсе.
 */

export interface Tab {
  key: string
  title: string
  /** Вкладка-терминал (дерево панелей) или вкладка-редактор файла. */
  kind: 'terminal' | 'editor'
  root: PaneNode
  activePaneId: string
  sftpOpen: boolean
  workspace: WorkspaceTool
  /** Для kind==='editor': какой файл и на какой сессии редактируется. */
  editor?: { sessionId: string; remotePath: string }
  editorDirty?: boolean
}

/** Что открыть в новой панели при сплите: локальный терминал или конкретный сервер. */
export type SplitChoice = { kind: 'local' } | { kind: 'ssh'; serverId: string; title: string }

export function uid(): string {
  return crypto.randomUUID()
}

export function serializedHasServer(node: SerializedPane, serverId: string): boolean {
  if (node.t === 'leaf') return node.serverId === serverId
  return (
    serializedHasServer(node.children[0], serverId) ||
    serializedHasServer(node.children[1], serverId)
  )
}

export function tabFromSaved(st: SerializedTab, restoreWorkspace: boolean): Tab {
  const root = deserializePane(st.root)
  const workspace = restoreWorkspace ? parseWorkspaceTool(st.workspace, st.sftpOpen) : 'terminal'
  const legacyWs = st.workspace as string | undefined
  const sftpOpen = restoreWorkspace && (!!st.sftpOpen || legacyWs === 'files')
  return {
    key: uid(),
    title: st.title,
    kind: 'terminal',
    root,
    activePaneId: firstLeaf(root).id,
    sftpOpen,
    workspace
  }
}

/** Что известно на момент запуска. */
export interface RestoreInput {
  /** Восстанавливать вкладки целиком. */
  restoreTabs: boolean
  /** Восстанавливать доп. окна (SFTP, логи) — и вкладки, которые им нужны. */
  restoreAux: boolean
  saved: SerializedTab[]
  auxWindows: SavedAuxWindow[]
  servers: ServerConfig[]
}

/**
 * Какие вкладки открыть при запуске.
 *
 * Три случая, и второй — неочевидный:
 *
 * 1. Просили восстановить вкладки — открываем всё сохранённое как было.
 * 2. Вкладки не просили, но просили доп. окна. Окно SFTP или логов живёт поверх
 *    SSH-сессии, а сессия берётся из вкладки. Значит вкладки всё равно нужны — но не все
 *    подряд, а ровно те, без которых окно не откроется. Если подходящей сохранённой
 *    вкладки нет, собираем минимальную из профиля сервера.
 * 3. Не просили ничего — пусто, дальше решает вызывающий (открыть локальный терминал).
 *
 * Функция чистая: ничего не читает и не пишет, поэтому её поведение можно проверить.
 */
export function planRestore(input: RestoreInput): Tab[] {
  const { restoreTabs, restoreAux, saved, auxWindows, servers } = input

  if (restoreTabs && saved.length) {
    return saved.map((st) => tabFromSaved(st, true))
  }
  if (!restoreAux) return []

  const out: Tab[] = []
  const used = new Set<string>()

  // Сначала вкладки, у которых сам пользователь оставил открытым файловый менеджер.
  for (const st of saved) {
    const legacyWs = st.workspace as string | undefined
    if (!st.sftpOpen && legacyWs !== 'files' && legacyWs !== 'resources') continue
    out.push(tabFromSaved(st, true))
    for (const l of allLeaves(deserializePane(st.root))) {
      if (l.serverId) used.add(l.serverId)
    }
  }

  // Затем — под каждое сохранённое окно, которому вкладки ещё не досталось.
  for (const w of auxWindows) {
    if (!w.serverId || used.has(w.serverId)) continue
    const st = saved.find((x) => serializedHasServer(x.root, w.serverId!))
    if (st) {
      out.push(tabFromSaved(st, false))
    } else {
      const srv = servers.find((x) => x.id === w.serverId)
      if (srv) {
        const leaf = makeLeaf(paneKindOf(srv), srv.name, srv.id)
        out.push({
          key: uid(),
          title: srv.name,
          kind: 'terminal',
          root: leaf,
          activePaneId: leaf.id,
          sftpOpen: false,
          workspace: 'terminal'
        })
      }
    }
    used.add(w.serverId)
  }

  return out
}

/** Вкладка, в которой живёт эта сессия. */
export function findTabKeyBySession(tabs: Tab[], sessionId: string): string | undefined {
  for (const t of tabs) {
    if (t.kind !== 'terminal') continue
    for (const l of allLeaves(t.root)) {
      if (l.sessionId === sessionId) return t.key
    }
  }
  return undefined
}

/** Что вернуть в главное окно из откреплённого. */
export interface ReattachRequest {
  sessionId: string
  serverId?: string
  title: string
  workspace: WorkspaceTool
  sftpOpen: boolean
  kind: PaneKind
}

/**
 * Куда положить возвращаемую вкладку.
 *
 * Два случая, и различать их обязательно. Обычно вкладки в главном окне уже нет — она
 * уехала вместе с окном, и её надо создать заново. Но бывает и наоборот: из откреплённого
 * окна вернули не всю вкладку, а только панель (Docker, логи, файлы), и вкладка с этой
 * сессией никуда не девалась. Тогда создавать вторую нельзя — получилось бы две вкладки
 * на одну сессию, и любая из них при закрытии унесла бы её у соседа.
 */
export function planReattach(
  tabs: Tab[],
  req: ReattachRequest
): { tabs: Tab[]; activeKey: string } {
  const existing = findTabKeyBySession(tabs, req.sessionId)
  if (existing) {
    return {
      tabs: tabs.map((t) =>
        t.key === existing
          ? { ...t, workspace: req.workspace, sftpOpen: req.sftpOpen, title: req.title }
          : t
      ),
      activeKey: existing
    }
  }

  const leaf = makeLeaf(req.kind, req.title, req.serverId)
  // Сессия уже поднята и живёт — вкладка создаётся сразу подключённой, без нового коннекта.
  const root: PaneLeaf = { ...leaf, sessionId: req.sessionId, status: 'connected' }
  const key = uid()
  return {
    tabs: [
      ...tabs,
      {
        key,
        title: req.title,
        kind: 'terminal',
        root,
        activePaneId: root.id,
        sftpOpen: req.sftpOpen,
        workspace: req.workspace
      }
    ],
    activeKey: key
  }
}

/**
 * Кому уйдёт ввод при включённом broadcast.
 *
 * Правило одно и оно про безопасность, а не про удобство: **только панели той же вкладки**,
 * где человек печатает. Соблазн разослать по всем вкладкам понятен — но вкладки обычно
 * держат разные окружения, и один такой «удобный» ввод отправляет команду в прод из
 * вкладки, открытой на тесте. Отменить это нельзя.
 *
 * Источник исключаем: он получит свой ввод сам, обычным путём.
 */
export function broadcastTargets(tabs: Tab[], fromSessionId: string): string[] {
  const tab = tabs.find((t) => allLeaves(t.root).some((l) => l.sessionId === fromSessionId))
  if (!tab) return []
  return allLeaves(tab.root)
    .map((l) => l.sessionId)
    .filter((id): id is string => !!id && id !== fromSessionId)
}

/**
 * Закрыть панель.
 *
 * Две тонкости, обе видны только на краях:
 *
 * - **последняя панель закрывает вкладку целиком** — пустая вкладка без единой панели
 *   выглядела бы сломанной;
 * - если закрыли активную панель, активной становится первая из оставшихся, иначе вкладка
 *   ссылается на панель, которой уже нет.
 */
export function closePaneIn(
  tabs: Tab[],
  tabKey: string,
  paneId: string
): { tabs: Tab[]; closedTab: boolean } {
  const next: Tab[] = []
  let closedTab = false
  for (const t of tabs) {
    if (t.key !== tabKey) {
      next.push(t)
      continue
    }
    const root = removeLeaf(t.root, paneId)
    if (!root) {
      closedTab = true
      continue
    }
    const stillActive = findLeaf(root, t.activePaneId)
    next.push({ ...t, root, activePaneId: stillActive ? t.activePaneId : firstLeaf(root).id })
  }
  return { tabs: next, closedTab }
}

/**
 * За какой панелью следует рельс инструментов (Docker, логи, процессы).
 *
 * Вкладка может держать несколько панелей, в том числе на разных серверах. Рельс один,
 * значит надо выбрать — и цена ошибки здесь выше, чем кажется: Docker покажет контейнеры
 * не того сервера, а команда «перезапустить» уйдёт не туда, куда человек смотрит.
 *
 * Порядок такой:
 *
 * 1. **Активная панель, если она SSH.** Самое честное правило: инструменты относятся к
 *    тому, на что человек сейчас смотрит.
 * 2. Иначе первая подключённая SSH-панель — активной может быть локальный терминал, а
 *    инструменты всё равно должны к чему-то относиться.
 * 3. Иначе первая SSH-панель вообще, даже неподключённая: рельс тогда покажет статус и
 *    кнопку переподключения, а не исчезнет молча.
 *
 * ⚠ Случай «активна не-SSH панель, а SSH-панелей несколько и все подключены» разрешается
 * произвольно — берём первую. Это осознанная неопределённость, а не продуманное правило:
 * угадать намерение тут нечем. Если станет мешать, лечится не здесь, а в интерфейсе —
 * явным выбором, к какой панели относятся инструменты.
 */
export function sshLeafForTools(tab: Tab): PaneLeaf | undefined {
  const active = findLeaf(tab.root, tab.activePaneId)
  if (active?.kind === 'ssh') return active
  const ssh = allLeaves(tab.root).filter((l) => l.kind === 'ssh')
  return ssh.find((l) => l.status === 'connected') ?? ssh[0]
}

/** Переставить вкладку на место другой. */
export function reorderTabs(tabs: Tab[], fromKey: string, toKey: string): Tab[] {
  const from = tabs.findIndex((t) => t.key === fromKey)
  const to = tabs.findIndex((t) => t.key === toKey)
  // Неизвестный ключ или перенос на себя — не повод перетасовывать список.
  if (from < 0 || to < 0 || from === to) return tabs
  const next = [...tabs]
  const [moved] = next.splice(from, 1)
  next.splice(to, 0, moved)
  return next
}

/** Новая вкладка подключения к серверу. */
export function makeServerTab(server: ServerConfig): Tab {
  const leaf = makeLeaf(paneKindOf(server), server.name, server.id)
  return {
    key: uid(),
    title: server.name,
    kind: 'terminal',
    root: leaf,
    activePaneId: leaf.id,
    sftpOpen: false,
    workspace: 'terminal'
  }
}

/** Новая вкладка локального терминала. */
export function makeLocalTab(): Tab {
  const leaf = makeLeaf('local', 'Локальный терминал')
  return {
    key: uid(),
    title: 'Локальный терминал',
    kind: 'terminal',
    root: leaf,
    activePaneId: leaf.id,
    sftpOpen: false,
    workspace: 'terminal'
  }
}

/** Уже открытый редактор этого файла на этой сессии. */
export function findEditorTab(tabs: Tab[], sessionId: string, remotePath: string): Tab | undefined {
  return tabs.find(
    (t) => t.kind === 'editor' && t.editor?.sessionId === sessionId && t.editor?.remotePath === remotePath
  )
}

/** Новая вкладка встроенного редактора удалённого файла. */
export function makeEditorTab(sessionId: string, remotePath: string): Tab {
  const fileName = remotePath.split('/').pop() || remotePath
  const leaf = makeLeaf('local', fileName)
  return {
    key: uid(),
    title: fileName,
    kind: 'editor',
    root: leaf,
    activePaneId: leaf.id,
    sftpOpen: false,
    workspace: 'terminal',
    editor: { sessionId, remotePath }
  }
}

/**
 * Разделить активную панель: в новой — выбранный сервер или локальный терминал.
 *
 * Вид панели берём из профиля: иначе сервер с COM-портом или telnet открылся бы как SSH
 * и падал на первом шаге подключения.
 */
export function planSplitPane(
  tab: Tab,
  servers: ServerConfig[],
  dir: 'row' | 'col',
  choice: SplitChoice
): Tab {
  const cur = findLeaf(tab.root, tab.activePaneId)
  if (!cur) return tab
  let fresh: PaneLeaf
  if (choice.kind === 'local') {
    fresh = makeLeaf('local', 'Локальный терминал')
  } else {
    const srv = servers.find((x) => x.id === choice.serverId)
    fresh = makeLeaf(srv ? paneKindOf(srv) : 'ssh', choice.title, choice.serverId)
  }
  return { ...tab, root: splitLeaf(tab.root, cur.id, dir, fresh), activePaneId: fresh.id }
}

export type DetachBlockReason = 'not-terminal' | 'multi-pane' | 'not-connected' | 'no-server'

export type DetachCheck =
  | { ok: true; leaf: PaneLeaf & { sessionId: string } }
  | { ok: false; reason: DetachBlockReason }

/** Можно ли открепить вкладку в отдельное окно. */
export function canDetachTab(tab: Tab): DetachCheck {
  if (tab.kind !== 'terminal') return { ok: false, reason: 'not-terminal' }
  const leaves = allLeaves(tab.root)
  if (leaves.length > 1) return { ok: false, reason: 'multi-pane' }
  const leaf = leaves[0]
  if (!leaf?.sessionId || leaf.status !== 'connected') return { ok: false, reason: 'not-connected' }
  if (leaf.kind === 'ssh' && !leaf.serverId) return { ok: false, reason: 'no-server' }
  return { ok: true, leaf: { ...leaf, sessionId: leaf.sessionId } }
}

export function toggleSftpOnTab(tabs: Tab[], key: string): Tab[] {
  return tabs.map((t) => {
    if (t.key !== key) return t
    if (t.sftpOpen) return { ...t, sftpOpen: false }
    return { ...t, sftpOpen: true, workspace: 'terminal' }
  })
}

export function openSftpOnTab(tabs: Tab[], key: string): Tab[] {
  return tabs.map((t) => (t.key === key ? { ...t, sftpOpen: true, workspace: 'terminal' } : t))
}

export function setTabWorkspace(tabs: Tab[], key: string, tool: WorkspaceTool): Tab[] {
  return tabs.map((t) => (t.key === key ? { ...t, workspace: tool } : t))
}

/** Что сохранить в раскладке для восстановления терминальных вкладок. */
export function tabsForLayoutPersist(tabs: Tab[]): SerializedTab[] {
  return tabs
    .filter((t) => t.kind === 'terminal')
    .map((t) => ({
      title: t.title,
      root: serializePane(t.root),
      sftpOpen: t.sftpOpen,
      workspace: t.workspace
    }))
}

/** Следующая вкладка для горячей клавиши переключения. */
export function nextTabKey(tabs: Tab[], activeKey: string | null, dir: 1 | -1): string | null {
  if (tabs.length < 2) return null
  const idx = tabs.findIndex((t) => t.key === activeKey)
  const base = idx < 0 ? 0 : idx
  return tabs[(base + dir + tabs.length) % tabs.length].key
}

/** Следующая панель внутри вкладки для горячей клавиши фокуса. */
export function nextActivePaneId(tab: Tab, dir: 1 | -1): string | null {
  if (tab.kind !== 'terminal') return null
  const leaves = allLeaves(tab.root)
  if (leaves.length < 2) return null
  const idx = leaves.findIndex((l) => l.id === tab.activePaneId)
  const base = idx < 0 ? 0 : idx
  return leaves[(base + dir + leaves.length) % leaves.length].id
}

export type ReattachWorkspacePlan =
  | { action: 'focus'; key: string; workspace: WorkspaceTool }
  | { action: 'insert'; payload: ReattachRequest }

/** Куда положить панель рабочего места, вернувшуюся из откреплённого окна. */
export function planReattachWorkspace(tabs: Tab[], p: ReattachWorkspacePayload): ReattachWorkspacePlan {
  const existing = findTabKeyBySession(tabs, p.sessionId)
  if (existing) return { action: 'focus', key: existing, workspace: p.tool }
  return {
    action: 'insert',
    payload: {
      sessionId: p.sessionId,
      serverId: p.serverId,
      title: p.title,
      workspace: p.tool,
      sftpOpen: false,
      kind: 'ssh'
    }
  }
}

export type ReattachSftpPlan =
  | { action: 'focus'; key: string }
  | { action: 'insert'; payload: ReattachRequest }

/** Куда положить SFTP, вернувшийся из откреплённого окна. */
export function planReattachSftp(tabs: Tab[], p: ReattachSftpPayload, serverName?: string): ReattachSftpPlan {
  const existing = findTabKeyBySession(tabs, p.sessionId)
  if (existing) return { action: 'focus', key: existing }
  return {
    action: 'insert',
    payload: {
      sessionId: p.sessionId,
      serverId: p.serverId,
      title: serverName ?? 'SSH',
      workspace: 'terminal',
      sftpOpen: true,
      kind: 'ssh'
    }
  }
}
