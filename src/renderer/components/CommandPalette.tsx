import { useMemo, useState } from 'react'

export interface PaletteItem {
  id: string
  label: string
  hint?: string
  icon?: string
  /** Категория для группировки/иконки. */
  group: string
  run: () => void
}

interface Props {
  items: PaletteItem[]
  onClose: () => void
}

/** Простой фаззи-матч: символы запроса встречаются по порядку в строке. */
function fuzzy(query: string, text: string): boolean {
  if (!query) return true
  const q = query.toLowerCase()
  const t = text.toLowerCase()
  let i = 0
  for (const ch of t) {
    if (ch === q[i]) i++
    if (i === q.length) return true
  }
  return false
}

export function CommandPalette({ items, onClose }: Props): JSX.Element {
  const [query, setQuery] = useState('')
  const [sel, setSel] = useState(0)

  const filtered = useMemo(() => {
    const list = items.filter((it) => fuzzy(query, it.label + ' ' + (it.hint ?? '')))
    return list.slice(0, 50)
  }, [items, query])

  const run = (item: PaletteItem | undefined): void => {
    if (!item) return
    item.run()
    onClose()
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        <input
          className="palette-input"
          autoFocus
          placeholder="Поиск сервера или команды…"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
            setSel(0)
          }}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') {
              e.preventDefault()
              setSel((s) => Math.min(s + 1, filtered.length - 1))
            } else if (e.key === 'ArrowUp') {
              e.preventDefault()
              setSel((s) => Math.max(s - 1, 0))
            } else if (e.key === 'Enter') {
              e.preventDefault()
              run(filtered[sel])
            } else if (e.key === 'Escape') {
              onClose()
            }
          }}
        />
        <div className="palette-list">
          {filtered.length === 0 && <div className="hint" style={{ padding: '10px 14px' }}>Ничего не найдено.</div>}
          {filtered.map((it, i) => (
            <div
              key={it.id}
              className={'palette-item' + (i === sel ? ' selected' : '')}
              onMouseEnter={() => setSel(i)}
              onClick={() => run(it)}
            >
              <span className="palette-icon">{it.icon ?? '•'}</span>
              <span className="palette-label">{it.label}</span>
              {it.hint && <span className="palette-hint">{it.hint}</span>}
              <span className="palette-group">{it.group}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}
