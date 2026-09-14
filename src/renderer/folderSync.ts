/**
 * Сравнение своей папки с папкой на сервере: что показать и что заливать.
 *
 * Решение по каждому файлу принимает бэкенд (`foldersync.rs`) - здесь только то, что из
 * этого решения следует для окна: счётчики, список к заливке и порядок действий. Вынесено
 * и покрыто тестами, потому что ошибка здесь - это залитый не туда файл.
 */

export type SyncKind = 'changed' | 'new' | 'remoteNewer' | 'unsure' | 'remoteOnly' | 'same'

export interface SyncItem {
  rel: string
  kind: SyncKind
  localSize?: number | null
  remoteSize?: number | null
  localMtime?: number | null
  remoteMtime?: number | null
}

export interface SyncPlan {
  localRoot: string
  remoteRoot: string
  /** Знаем ли время правки на сервере. По SCP - нет: одинаковый размер тогда - «не определить». */
  timeKnown: boolean
  items: SyncItem[]
  /** Каталоги, которые на сервере уже есть, - путями от корня сравнения. */
  remoteDirs: string[]
  refused: { rel: string; why: string }[]
}

export const KIND_LABEL: Record<SyncKind, string> = {
  changed: 'изменён',
  new: 'новый',
  remoteNewer: 'на сервере новее',
  unsure: 'не определить',
  remoteOnly: 'только на сервере',
  same: 'совпадает'
}

export const KIND_ORDER: SyncKind[] = ['changed', 'new', 'remoteNewer', 'unsure', 'remoteOnly', 'same']

export function countKinds(items: SyncItem[]): Record<SyncKind, number> {
  const out: Record<SyncKind, number> = { changed: 0, new: 0, remoteNewer: 0, unsure: 0, remoteOnly: 0, same: 0 }
  for (const it of items) out[it.kind]++
  return out
}

/**
 * Что заливать. «На сервере новее» - только по явному согласию: там правили позже, и
 * заливка затёрла бы чужую правку. «Не определить» - тоже: размер тот же, а одинаково ли
 * содержимое, не проверить.
 */
export function toUpload(items: SyncItem[], includeRemoteNewer: boolean, includeUnsure = false): SyncItem[] {
  return items.filter(
    (it) =>
      it.kind === 'changed' ||
      it.kind === 'new' ||
      (includeRemoteNewer && it.kind === 'remoteNewer') ||
      (includeUnsure && it.kind === 'unsure')
  )
}

/**
 * Что из заливаемого изменилось на сервере после сравнения: в свежем сравнении у файла
 * другое время правки или размер на сервере, он там появился - или своего файла больше нет.
 * По старому плану такое не заливаем: согласие было на другую версию.
 */
export function changedSincePlan(upload: SyncItem[], fresh: SyncPlan): string[] {
  const now = new Map(fresh.items.map((it) => [it.rel, it]))
  return upload
    .filter((it) => {
      const cur = now.get(it.rel)
      if (!cur) return true
      return (
        (cur.remoteSize ?? null) !== (it.remoteSize ?? null) || (cur.remoteMtime ?? null) !== (it.remoteMtime ?? null)
      )
    })
    .map((it) => it.rel)
}

export function joinRemoteRel(root: string, rel: string): string {
  return rel ? `${root.replace(/\/+$/, '')}/${rel}` : root
}

/** Путь своего файла. Разделитель - тот, что уже в корне: на Windows обратная черта. */
export function joinLocalRel(root: string, rel: string): string {
  const sep = root.includes('\\') && !root.includes('/') ? '\\' : '/'
  return `${root.replace(/[\\/]+$/, '')}${sep}${rel.split('/').join(sep)}`
}

function parentRel(rel: string): string {
  const i = rel.lastIndexOf('/')
  return i < 0 ? '' : rel.slice(0, i)
}

export interface UploadSteps {
  /** Каталоги на сервере, которых нет, - от внешних к внутренним. */
  mkdirs: string[]
  /** Заливка группами: в какой каталог сервера и какие свои файлы. */
  groups: { remoteDir: string; localPaths: string[] }[]
  files: number
}

/**
 * Порядок действий для заливки.
 *
 * Каталоги создаются заранее и по одному уровню: заливка по SCP сама каталогов не создаёт,
 * а файл в несуществующий каталог просто не ляжет.
 */
export function uploadSteps(plan: SyncPlan, includeRemoteNewer: boolean, includeUnsure = false): UploadSteps {
  const files = toUpload(plan.items, includeRemoteNewer, includeUnsure)
  const have = new Set(plan.remoteDirs)
  const need = new Set<string>()
  for (const f of files) {
    const parts = f.rel.split('/')
    for (let i = 1; i < parts.length; i++) {
      const d = parts.slice(0, i).join('/')
      if (!have.has(d)) need.add(d)
    }
  }
  const depth = (d: string): number => d.split('/').length
  const mkdirs = [...need]
    .sort((a, b) => depth(a) - depth(b) || a.localeCompare(b))
    .map((d) => joinRemoteRel(plan.remoteRoot, d))

  const byDir = new Map<string, string[]>()
  for (const f of files) {
    const dir = joinRemoteRel(plan.remoteRoot, parentRel(f.rel))
    if (!byDir.has(dir)) byDir.set(dir, [])
    byDir.get(dir)!.push(joinLocalRel(plan.localRoot, f.rel))
  }
  return {
    mkdirs,
    groups: [...byDir.entries()].map(([remoteDir, localPaths]) => ({ remoteDir, localPaths })),
    files: files.length
  }
}
