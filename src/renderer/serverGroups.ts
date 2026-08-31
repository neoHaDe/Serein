import type { ServerConfig } from '../shared/types'

/** Группы из настроек плюс те, что встречаются у серверов (могли приехать импортом). */
export function mergeGroupNames(groupOrder: string[], servers: ServerConfig[]): string[] {
  const fromServers = servers
    .map((s) => s.group?.trim())
    .filter((g): g is string => !!g)
  return [...new Set([...groupOrder, ...fromServers])]
}

/** Новый порядок серверов в группе после перетаскивания одного из них. */
export function planDropServer(
  servers: ServerConfig[],
  serverId: string,
  group: string,
  index?: number
): { id: string; group: string; order: number }[] | null {
  const moved = servers.find((s) => s.id === serverId)
  if (!moved) return null
  const target = servers
    .filter((s) => s.id !== serverId && (s.group?.trim() || '') === group)
    .sort((a, b) => (a.order ?? 0) - (b.order ?? 0) || a.name.localeCompare(b.name))
  const at = index === undefined ? target.length : Math.max(0, Math.min(index, target.length))
  target.splice(at, 0, moved)
  return target.map((s, i) => ({ id: s.id, group, order: i }))
}

/** Применить пересчитанный порядок к списку серверов для немедленного отображения. */
export function applyDropServerOrder(
  servers: ServerConfig[],
  order: { id: string; group: string; order: number }[]
): ServerConfig[] {
  const byId = new Map(order.map((o) => [o.id, o]))
  return servers.map((s) => {
    const next = byId.get(s.id)
    return next ? { ...s, group: next.group, order: next.order } : s
  })
}

/** Переставить группу в списке на одну позицию влево или вправо. */
export function planMoveGroup(
  groupOrder: string[],
  fallbackGroups: string[],
  group: string,
  dir: -1 | 1
): string[] | null {
  const cur = [...(groupOrder.length ? groupOrder : fallbackGroups)]
  const i = cur.indexOf(group)
  const j = i + dir
  if (i < 0 || j < 0 || j >= cur.length) return null
  ;[cur[i], cur[j]] = [cur[j], cur[i]]
  return cur
}
