import type { SessionStatus, SerializedPane } from '../shared/types'

/** Лист дерева — один терминал (SSH или локальный). */
export interface PaneLeaf {
  type: 'leaf'
  id: string
  kind: 'ssh' | 'local'
  serverId?: string
  title: string
  sessionId?: string
  status: SessionStatus['status']
  statusMsg?: string
  /** Счётчик пересозданий (reconnect → remount терминала). */
  gen: number
}

/** Разделитель — два потомка по горизонтали (row) или вертикали (col). */
export interface PaneSplit {
  type: 'split'
  id: string
  dir: 'row' | 'col'
  children: [PaneNode, PaneNode]
  /** Проценты размеров потомков, в сумме ~100. */
  sizes: [number, number]
}

export type PaneNode = PaneLeaf | PaneSplit

function uid(): string {
  return crypto.randomUUID()
}

export function makeLeaf(kind: 'ssh' | 'local', title: string, serverId?: string): PaneLeaf {
  return { type: 'leaf', id: uid(), kind, serverId, title, status: 'connecting', gen: 0 }
}

export function firstLeaf(node: PaneNode): PaneLeaf {
  return node.type === 'leaf' ? node : firstLeaf(node.children[0])
}

export function allLeaves(node: PaneNode): PaneLeaf[] {
  return node.type === 'leaf' ? [node] : [...allLeaves(node.children[0]), ...allLeaves(node.children[1])]
}

export function findLeaf(node: PaneNode, id: string): PaneLeaf | undefined {
  if (node.type === 'leaf') return node.id === id ? node : undefined
  return findLeaf(node.children[0], id) ?? findLeaf(node.children[1], id)
}

type LeafPatch = Partial<PaneLeaf> | ((l: PaneLeaf) => Partial<PaneLeaf>)

/** Возвращает новое дерево с применённым патчем к листу по id. patch — объект или функция. */
export function updateLeaf(node: PaneNode, id: string, patch: LeafPatch): PaneNode {
  if (node.type === 'leaf') {
    if (node.id !== id) return node
    return { ...node, ...(typeof patch === 'function' ? patch(node) : patch) }
  }
  return {
    ...node,
    children: [updateLeaf(node.children[0], id, patch), updateLeaf(node.children[1], id, patch)]
  }
}

/** Патч листа по sessionId. */
export function updateLeafBySession(node: PaneNode, sessionId: string, patch: Partial<PaneLeaf>): PaneNode {
  if (node.type === 'leaf') return node.sessionId === sessionId ? { ...node, ...patch } : node
  return {
    ...node,
    children: [
      updateLeafBySession(node.children[0], sessionId, patch),
      updateLeafBySession(node.children[1], sessionId, patch)
    ]
  }
}

/** Делит лист targetId на split [target, newLeaf]. */
export function splitLeaf(node: PaneNode, targetId: string, dir: 'row' | 'col', newLeaf: PaneLeaf): PaneNode {
  if (node.type === 'leaf') {
    if (node.id !== targetId) return node
    return { type: 'split', id: uid(), dir, children: [node, newLeaf], sizes: [50, 50] }
  }
  return {
    ...node,
    children: [splitLeaf(node.children[0], targetId, dir, newLeaf), splitLeaf(node.children[1], targetId, dir, newLeaf)]
  }
}

/** Удаляет лист; родительский split схлопывается в оставшегося потомка. null — дерево пусто. */
export function removeLeaf(node: PaneNode, targetId: string): PaneNode | null {
  if (node.type === 'leaf') return node.id === targetId ? null : node
  const a = removeLeaf(node.children[0], targetId)
  const b = removeLeaf(node.children[1], targetId)
  if (a && b) return { ...node, children: [a, b] }
  return a ?? b // схлопываем split в выжившего потомка
}

/** Сериализует дерево панелей (без рантайм-полей) для сохранения раскладки. */
export function serializePane(node: PaneNode): SerializedPane {
  if (node.type === 'leaf') {
    return { t: 'leaf', kind: node.kind, serverId: node.serverId, title: node.title }
  }
  return {
    t: 'split',
    dir: node.dir,
    sizes: node.sizes,
    children: [serializePane(node.children[0]), serializePane(node.children[1])]
  }
}

/** Восстанавливает дерево панелей из сериализованного вида (со свежими id). */
export function deserializePane(node: SerializedPane): PaneNode {
  if (node.t === 'leaf') return makeLeaf(node.kind, node.title, node.serverId)
  return {
    type: 'split',
    id: uid(),
    dir: node.dir,
    sizes: node.sizes,
    children: [deserializePane(node.children[0]), deserializePane(node.children[1])]
  }
}

export function updateSplitSizes(node: PaneNode, splitId: string, sizes: [number, number]): PaneNode {
  if (node.type === 'leaf') return node
  if (node.id === splitId) return { ...node, sizes }
  return {
    ...node,
    children: [updateSplitSizes(node.children[0], splitId, sizes), updateSplitSizes(node.children[1], splitId, sizes)]
  }
}
