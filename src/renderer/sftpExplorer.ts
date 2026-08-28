/** Колонки SFTP-проводника: ширины, сортировка, тип файла. */
export type SftpColId = 'name' | 'ext' | 'mode' | 'size' | 'mtime'

export const SFTP_COL_ORDER: SftpColId[] = ['name', 'ext', 'mode', 'size', 'mtime']

export const SFTP_COL_LABEL: Record<SftpColId, string> = {
  name: 'Имя',
  ext: 'Тип',
  mode: 'Права',
  size: 'Размер',
  mtime: 'Дата изменения',
}

export const DEFAULT_SFTP_COL_ON: Record<SftpColId, boolean> = {
  name: true,
  ext: true,
  mode: true,
  size: true,
  mtime: true,
}

export const DEFAULT_SFTP_COL_WIDTH: Record<SftpColId, number> = {
  name: 200,
  ext: 64,
  mode: 52,
  size: 84,
  mtime: 136,
}

export const SFTP_COL_MIN: Record<SftpColId, number> = {
  name: 88,
  ext: 40,
  mode: 40,
  size: 56,
  mtime: 96,
}

export type SftpSortDir = 'asc' | 'desc'

export interface SftpSort {
  col: SftpColId
  dir: SftpSortDir
}

export const DEFAULT_SFTP_SORT: SftpSort = { col: 'name', dir: 'asc' }

export function mergeColOn(raw?: Partial<Record<SftpColId, boolean>>): Record<SftpColId, boolean> {
  return { ...DEFAULT_SFTP_COL_ON, ...raw, name: true }
}

export function mergeColWidths(raw?: Partial<Record<SftpColId, number>>): Record<SftpColId, number> {
  const out = { ...DEFAULT_SFTP_COL_WIDTH }
  if (!raw) return out
  for (const id of SFTP_COL_ORDER) {
    const n = raw[id]
    if (typeof n === 'number' && Number.isFinite(n)) out[id] = Math.max(SFTP_COL_MIN[id], Math.round(n))
  }
  return out
}

export function mergeSort(col?: string, dir?: string): SftpSort {
  const c = SFTP_COL_ORDER.includes(col as SftpColId) ? (col as SftpColId) : 'name'
  const d: SftpSortDir = dir === 'desc' ? 'desc' : 'asc'
  return { col: c, dir: d }
}

export function visibleCols(on: Record<SftpColId, boolean>, hideMode = false): SftpColId[] {
  return SFTP_COL_ORDER.filter((id) => on[id] && !(hideMode && id === 'mode'))
}

export function gridTemplate(cols: SftpColId[], widths: Record<SftpColId, number>): string {
  return cols
    .map((id) => (id === 'name' ? `minmax(${widths[id]}px, 1fr)` : `${widths[id]}px`))
    .join(' ')
}

/** Расширение; у папки — «папка». */
export function fileExt(name: string, isDir: boolean): string {
  if (isDir) return 'папка'
  const base = name.includes('/') ? name.slice(name.lastIndexOf('/') + 1) : name
  if (base.startsWith('.') && !base.slice(1).includes('.')) return ''
  const i = base.lastIndexOf('.')
  if (i <= 0) return ''
  return base.slice(i + 1).toLowerCase()
}

export function fmtMtime(ms: number): string {
  if (!ms) return ''
  const d = new Date(ms)
  if (Number.isNaN(d.getTime())) return ''
  return d.toLocaleString('ru-RU', {
    day: '2-digit',
    month: '2-digit',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export function nextSort(cur: SftpSort, col: SftpColId): SftpSort {
  if (cur.col !== col) {
    return { col, dir: col === 'mtime' || col === 'size' ? 'desc' : 'asc' }
  }
  return { col, dir: cur.dir === 'asc' ? 'desc' : 'asc' }
}

export interface SortableEntry {
  name: string
  type: string
  size: number
  mtime: number
  mode?: number
  linkType?: string | null
}

export function isDirLike(e: SortableEntry): boolean {
  return e.type === 'dir' || (e.type === 'link' && e.linkType === 'dir')
}

export function sortEntries<T extends SortableEntry>(list: T[], sort: SftpSort): T[] {
  const dirSign = sort.dir === 'asc' ? 1 : -1
  return [...list].sort((a, b) => {
    const ad = isDirLike(a)
    const bd = isDirLike(b)
    if (ad !== bd) return ad ? -1 : 1
    let c = 0
    switch (sort.col) {
      case 'ext':
        c = fileExt(a.name, ad).localeCompare(fileExt(b.name, bd), 'ru', { numeric: true })
        break
      case 'mode':
        c = (a.mode ?? 0) - (b.mode ?? 0)
        break
      case 'size':
        c = a.size - b.size
        break
      case 'mtime':
        c = a.mtime - b.mtime
        break
      default:
        c = a.name.localeCompare(b.name, 'ru', { numeric: true, sensitivity: 'base' })
    }
    if (c === 0) c = a.name.localeCompare(b.name, 'ru', { numeric: true, sensitivity: 'base' })
    return c * dirSign
  })
}

export function clickSelect(
  visible: string[],
  current: string[],
  clicked: string,
  mods: { ctrl: boolean; shift: boolean },
  anchor: string | null
): { selected: string[]; anchor: string } {
  if (mods.shift && anchor && visible.includes(anchor) && visible.includes(clicked)) {
    const a = visible.indexOf(anchor)
    const b = visible.indexOf(clicked)
    const [lo, hi] = a < b ? [a, b] : [b, a]
    const range = visible.slice(lo, hi + 1)
    if (mods.ctrl) {
      const set = new Set(current)
      for (const n of range) set.add(n)
      return { selected: visible.filter((n) => set.has(n)), anchor }
    }
    return { selected: range, anchor }
  }
  if (mods.ctrl) {
    const set = new Set(current)
    if (set.has(clicked)) set.delete(clicked)
    else set.add(clicked)
    return { selected: visible.filter((n) => set.has(n)), anchor: clicked }
  }
  return { selected: [clicked], anchor: clicked }
}

export function entryKind(e: { name: string; type: string; linkType?: string | null }): string {
  if (e.type === 'dir' || (e.type === 'link' && e.linkType === 'dir')) return 'Папка'
  if (e.type === 'link') return e.linkType === 'broken' ? 'Сломанная ссылка' : 'Символическая ссылка'
  const ext = fileExt(e.name, false)
  return ext ? 'Файл .' + ext : 'Файл'
}

export function fmtPerms(mode: number): string {
  const o = mode & 0o777
  let s = ''
  const bits = ['r', 'w', 'x']
  for (let i = 0; i < 9; i++) s += o & (1 << (8 - i)) ? bits[i % 3] : '-'
  return s + ' (' + o.toString(8).padStart(3, '0') + ')'
}
