import type { SortState } from '../tableSort'

/**
 * Заголовок столбца, по которому сортируют щелчком.
 *
 * Один и для таблицы процессов (`th`), и для сетки контейнеров (`span`): вид и поведение
 * обязаны совпадать, иначе в одной панели стрелка значит одно, а в соседней другое.
 */
export function SortHeader<K extends string>({
  as,
  label,
  sortKey,
  sort,
  onSort,
  className
}: {
  as: 'th' | 'span'
  label: string
  sortKey: K
  sort: SortState<K>
  onSort: (key: K) => void
  className?: string
}): JSX.Element {
  const active = sort.key === sortKey
  const Tag = as
  return (
    <Tag
      className={(className ? className + ' ' : '') + 'ws-th-sort' + (active ? ' on' : '')}
      role={as === 'span' ? 'columnheader' : undefined}
      aria-sort={active ? (sort.dir === 'asc' ? 'ascending' : 'descending') : 'none'}
      title={`Сортировать по столбцу «${label}»`}
      onClick={() => onSort(sortKey)}
    >
      {label}
      <span className="ws-sort-arrow">{active ? (sort.dir === 'asc' ? '▲' : '▼') : ''}</span>
    </Tag>
  )
}
