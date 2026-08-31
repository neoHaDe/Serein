import type {
  PaneKind,
  SavedAuxWindow,
  SerializedPane,
  SerializedTab,
  ServerConfig,
  WorkspaceTool
} from '../shared/types'
import { paneKindOf, parseWorkspaceTool } from '../shared/types'
import type { PaneLeaf, PaneNode } from './paneTree'
import { allLeaves, deserializePane, firstLeaf, makeLeaf } from './paneTree'

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
