/**
 * Именованные профили рабочего пространства: «Разработка», «Продакшн», «Логи».
 *
 * Профиль хранит то же, что восстановление вкладок при запуске: раскладку панелей, серверы
 * и открытые инструменты. Паролей и ключей в нём нет - только ссылки на серверы, поэтому
 * сервер, удалённый после сохранения, профиль не «воскрешает», а честно теряет.
 */
import type { SerializedLeaf, SerializedPane, SerializedTab, ServerConfig, WorkspaceProfile } from '../shared/types'

function leaves(node: SerializedPane): SerializedLeaf[] {
  return node.t === 'leaf' ? [node] : [...leaves(node.children[0]), ...leaves(node.children[1])]
}

function plural(n: number, one: string, few: string, many: string): string {
  const mod100 = n % 100
  if (mod100 >= 11 && mod100 <= 14) return many
  const mod10 = n % 10
  if (mod10 === 1) return one
  if (mod10 >= 2 && mod10 <= 4) return few
  return many
}

export interface ProfileInfo {
  tabs: number
  /** Имена серверов профиля без повторов, в порядке появления. */
  servers: string[]
  /** Панелей без сервера - локальные терминалы. */
  local: number
  /** Панелей на серверы, которых больше нет. */
  missing: number
}

export function profileInfo(p: WorkspaceProfile, servers: ServerConfig[]): ProfileInfo {
  const byId = new Map(servers.map((s) => [s.id, s.name]))
  const names: string[] = []
  let local = 0
  let missing = 0
  for (const tab of p.tabs) {
    for (const leaf of leaves(tab.root)) {
      if (!leaf.serverId) {
        local++
        continue
      }
      const name = byId.get(leaf.serverId)
      if (name === undefined) missing++
      else if (!names.includes(name)) names.push(name)
    }
  }
  return { tabs: p.tabs.length, servers: names, local, missing }
}

export function profileSummary(info: ProfileInfo): string {
  const parts = [`${info.tabs} ${plural(info.tabs, 'вкладка', 'вкладки', 'вкладок')}`]
  if (info.servers.length) {
    const shown = info.servers.slice(0, 4).join(', ')
    parts.push(info.servers.length > 4 ? `${shown} и ещё ${info.servers.length - 4}` : shown)
  }
  if (info.local) parts.push(`локальных терминалов: ${info.local}`)
  if (info.missing) parts.push(`удалённых серверов: ${info.missing}`)
  return parts.join(' · ')
}

function prune(node: SerializedPane, known: Set<string>): SerializedPane | null {
  if (node.t === 'leaf') return node.serverId && !known.has(node.serverId) ? null : node
  const a = prune(node.children[0], known)
  const b = prune(node.children[1], known)
  if (a && b) return { ...node, children: [a, b] }
  return a ?? b
}

/**
 * Вкладки профиля без панелей на удалённые серверы. Такая панель не откроется, а вкладка,
 * где кроме неё ничего нет, была бы пустой ошибкой подключения к тому, чего нет.
 */
export function withoutMissing(tabs: SerializedTab[], servers: ServerConfig[]): SerializedTab[] {
  const known = new Set(servers.map((s) => s.id))
  const out: SerializedTab[] = []
  for (const tab of tabs) {
    const root = prune(tab.root, known)
    if (root) out.push({ ...tab, root })
  }
  return out
}

/** Что не так с названием, или `null`. Одинаковые имена путали бы «открыть» с «перезаписать». */
export function nameProblem(name: string, profiles: WorkspaceProfile[], ownId?: string): string | null {
  const n = name.trim()
  if (!n) return 'Нужно название'
  if (n.length > 60) return 'Название длиннее 60 символов'
  const taken = profiles.some((p) => p.id !== ownId && p.name.trim().toLowerCase() === n.toLowerCase())
  return taken ? `Профиль «${n}» уже есть` : null
}
