/**
 * Сортировка таблиц по щелчку на заголовке и поиск по строкам.
 *
 * Общая для процессов и контейнеров: щелчок по «памяти» обязан значить одно и то же в любой
 * панели. Отсутствующее значение (у BusyBox нет загрузки процессора, у остановленного
 * контейнера - памяти) всегда внизу, в какую сторону ни сортируй: иначе по возрастанию
 * список начинался бы со строк прочерков.
 */

export type SortDir = 'asc' | 'desc'

export interface SortState<K extends string> {
  key: K
  dir: SortDir
}

export type Cell = string | number | null | undefined

/** Щелчок по заголовку: тот же столбец - сменить направление, другой - начать с `firstDir`. */
export function nextSort<K extends string>(cur: SortState<K>, key: K, firstDir: SortDir): SortState<K> {
  if (cur.key === key) return { key, dir: cur.dir === 'asc' ? 'desc' : 'asc' }
  return { key, dir: firstDir }
}

function absent(v: Cell): boolean {
  return v === null || v === undefined || v === '' || (typeof v === 'number' && Number.isNaN(v))
}

// `numeric`: «nginx-10» после «nginx-9», а не между «nginx-1» и «nginx-2».
const collator = new Intl.Collator('ru', { numeric: true, sensitivity: 'base' })

/** Отсортированная копия. При равенстве сохраняется прежний порядок строк. */
export function sortRows<T, K extends string>(rows: T[], sort: SortState<K>, cell: (row: T, key: K) => Cell): T[] {
  const sign = sort.dir === 'asc' ? 1 : -1
  return rows
    .map((row, i) => ({ row, i, v: cell(row, sort.key) }))
    .sort((a, b) => {
      const na = absent(a.v)
      const nb = absent(b.v)
      if (na || nb) return na === nb ? a.i - b.i : na ? 1 : -1
      const d =
        typeof a.v === 'number' && typeof b.v === 'number'
          ? a.v - b.v
          : collator.compare(String(a.v), String(b.v))
      return d !== 0 ? sign * d : a.i - b.i
    })
    .map((x) => x.row)
}

/** Поиск: каждое слово запроса должно найтись хотя бы в одном из полей строки. */
export function matchesQuery(fields: Cell[], query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return true
  const hay = fields.filter((f) => !absent(f)).map((f) => String(f).toLowerCase())
  return words.every((w) => hay.some((h) => h.includes(w)))
}

/** Доля из вывода Docker: «12.34%» - 12.34. */
export function parsePercent(s: string | undefined): number | null {
  const m = /^\s*(-?\d+(?:\.\d+)?)\s*%\s*$/.exec(s ?? '')
  return m ? Number(m[1]) : null
}

const UNITS: Record<string, number> = {
  b: 1,
  kb: 1e3,
  mb: 1e6,
  gb: 1e9,
  tb: 1e12,
  kib: 1024,
  mib: 1024 ** 2,
  gib: 1024 ** 3,
  tib: 1024 ** 4
}

/** Объём из вывода Docker в байтах: первое число строки вида «123.4MiB / 1.94GiB». */
export function parseBytes(s: string | undefined): number | null {
  const m = /^\s*(\d+(?:\.\d+)?)\s*([a-z]+)/i.exec(s ?? '')
  if (!m) return null
  const k = UNITS[m[2].toLowerCase()]
  return k === undefined ? null : Number(m[1]) * k
}

/** Запомненная сортировка таблицы. Хранилища может не быть - тогда порядок по умолчанию. */
export function loadSort<K extends string>(storageKey: string, keys: readonly K[], fallback: SortState<K>): SortState<K> {
  try {
    const v = JSON.parse(localStorage.getItem(storageKey) ?? 'null') as SortState<K> | null
    if (v && keys.includes(v.key) && (v.dir === 'asc' || v.dir === 'desc')) return { key: v.key, dir: v.dir }
  } catch {
    // Нет хранилища или в нём мусор - порядок по умолчанию.
  }
  return fallback
}

export function saveSort(storageKey: string, sort: SortState<string>): void {
  try {
    localStorage.setItem(storageKey, JSON.stringify(sort))
  } catch {
    // Не запомнили - не беда: сортировка в этом окне всё равно работает.
  }
}
