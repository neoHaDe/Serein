import type { LocalEntry, SftpEntry } from '../shared/types'
import { entryKind, fmtMtime, fmtPerms } from './sftpExplorer'

/** Разбивает абсолютный remote-путь на сегменты-крошки: [{label, path}]. */
export function remoteCrumbs(path: string): { label: string; path: string }[] {
  if (!path.startsWith('/')) return [] // относительный путь (напр. '.') - крошки не строим
  const parts = path.split('/').filter(Boolean)
  const crumbs = [{ label: '/', path: '/' }]
  let acc = ''
  for (const p of parts) {
    acc += '/' + p
    crumbs.push({ label: p, path: acc })
  }
  return crumbs
}

export function fmtSize(n: number): string {
  if (n < 1024) return `${n} Б`
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} КБ`
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} МБ`
  return `${(n / 1024 ** 3).toFixed(2)} ГБ`
}

export function fmtSpeed(bps: number | undefined): string {
  if (!bps || !isFinite(bps) || bps < 256) return ''
  return fmtSize(bps) + '/s'
}

export function fmtEta(size: number, transferred: number, bps: number | undefined): string {
  if (!bps || bps < 1024 || !size || transferred >= size) return ''
  const sec = Math.round((size - transferred) / bps)
  if (sec < 1) return ''
  if (sec < 60) return `${sec}с`
  return `${Math.floor(sec / 60)}м ${sec % 60}с`
}

export function fmtMode(mode: number): string {
  return (mode & 0o777).toString(8).padStart(3, '0')
}

export function isHiddenName(name: string): boolean {
  return name.startsWith('.') && name !== '.' && name !== '..'
}

export function filterEntries<T extends { name: string }>(entries: T[], showHidden: boolean, query: string): T[] {
  const q = query.trim().toLowerCase()
  return entries.filter((e) => {
    if (!showHidden && isHiddenName(e.name)) return false
    if (q && !e.name.toLowerCase().includes(q)) return false
    return true
  })
}

export function isDirEntry(e: SftpEntry): boolean {
  return e.type === 'dir' || (e.type === 'link' && e.linkType === 'dir')
}

export function isFileLike(e: SftpEntry): boolean {
  return e.type === 'file' || (e.type === 'link' && e.linkType !== 'dir')
}

export function parentOfRemote(path: string): string {
  if (path === '/' || path === '') return '/'
  const trimmed = path.replace(/\/+$/, '')
  const idx = trimmed.lastIndexOf('/')
  return idx <= 0 ? '/' : trimmed.slice(0, idx)
}

/** Родитель пути любой из двух систем - для повтора передачи туда же, откуда она шла. */
export function parentOfAny(p: string): string {
  const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'))
  return i <= 0 ? p : p.slice(0, i)
}

export function joinRemote(dir: string, name: string): string {
  return dir.endsWith('/') ? dir + name : dir + '/' + name
}

export function joinLocal(dir: string, name: string): string {
  if (dir.endsWith('/') || dir.endsWith('\\')) return dir + name
  const sep = dir.includes('\\') ? '\\' : '/'
  return dir + sep + name
}

/** Файл из временного каталога перетаскивания: после него списки обновлять незачем. */
export function isSereinDnd(p: string): boolean {
  return p.replace(/\\/g, '/').toLowerCase().includes('/serein-dnd/')
}

export function localBaseName(p: string): string {
  const n = p.replace(/\\/g, '/').replace(/\/+$/, '')
  const i = n.lastIndexOf('/')
  return i >= 0 ? n.slice(i + 1) : n
}

/** Последний замер передачи: когда, сколько байт и какая была скорость. */
export interface RateSample {
  t: number
  b: number
  bps: number
}

/**
 * Скорость передачи по очередному событию - сглаженная, чтобы цифра не прыгала.
 *
 * `sample`: новый замер, `undefined` - оставить прежний, `null` - забыть (передача не идёт).
 * Замер обновляется не чаще раза в 0,2 с: события приходят пачками, и мгновенная скорость по
 * двум соседним была бы шумом.
 */
export function nextRate(
  prev: RateSample | undefined,
  state: string,
  transferred: number,
  now: number
): { bps: number; sample: RateSample | null | undefined } {
  if (state !== 'active') return { bps: 0, sample: null }
  if (!prev) return { bps: 0, sample: { t: now, b: transferred, bps: 0 } }
  if (transferred <= prev.b) return { bps: prev.bps, sample: undefined }
  const dt = (now - prev.t) / 1000
  if (dt < 0.2) return { bps: prev.bps, sample: undefined }
  const inst = (transferred - prev.b) / dt
  const bps = prev.bps > 0 ? prev.bps * 0.55 + inst * 0.45 : inst
  return { bps, sample: { t: now, b: transferred, bps } }
}

/** Лист «Свойства»: заголовок и строки «что - значение». */
export interface PropsData {
  title: string
  rows: { k: string; v: string }[]
}

function summary(count: number, dirs: number, location: string, total: number): PropsData {
  return {
    title: count + ' элементов',
    rows: [
      { k: 'Выделено', v: String(count) },
      { k: 'Папок', v: String(dirs) },
      { k: 'Файлов', v: String(count - dirs) },
      { k: 'Расположение', v: location },
      { k: 'Суммарный размер', v: fmtSize(total) },
    ],
  }
}

/** Свойства выделенного на сервере: одна запись - подробно, несколько - сводкой. */
export function remoteProps(items: SftpEntry[], location: string): PropsData {
  if (items.length === 1) {
    const e = items[0]
    const rows = [
      { k: 'Имя', v: e.name },
      { k: 'Тип', v: entryKind(e) },
      { k: 'Расположение', v: location },
      { k: 'Размер', v: fmtSize(e.size) },
      { k: 'Изменён', v: fmtMtime(e.mtime) },
      { k: 'Права', v: fmtPerms(e.mode) },
    ]
    if (e.type === 'link' && e.target) rows.push({ k: 'Ссылка', v: e.target })
    return { title: 'Свойства', rows }
  }
  const total = items.reduce((s, e) => s + (e.size || 0), 0)
  return summary(items.length, items.filter(isDirEntry).length, location, total)
}

/** То же для своей машины: прав и ссылок здесь не показываем. */
export function localProps(items: LocalEntry[], location: string): PropsData {
  if (items.length === 1) {
    const e = items[0]
    return {
      title: 'Свойства',
      rows: [
        { k: 'Имя', v: e.name },
        { k: 'Тип', v: entryKind(e) },
        { k: 'Расположение', v: location },
        { k: 'Размер', v: fmtSize(e.size) },
        { k: 'Изменён', v: fmtMtime(e.mtime) },
      ],
    }
  }
  const total = items.reduce((s, e) => s + (e.size || 0), 0)
  return summary(items.length, items.filter((e) => e.type === 'dir').length, location, total)
}
