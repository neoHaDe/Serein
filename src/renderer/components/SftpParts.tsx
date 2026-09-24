import { useLayoutEffect, useRef, useState } from 'react'
import { Icon } from './Icon'
import { SFTP_COL_LABEL, type SftpColId } from '../sftpExplorer'

// Мелкие части панели файлов: вопрос о замене, шапка столбцов, меню и лист свойств.

export function OverwriteAsk({
  names,
  onYes,
  onNo,
}: {
  names: string[]
  onYes: () => void
  onNo: () => void
}): JSX.Element {
  const msg =
    names.length === 1
      ? `«${names[0]}» уже есть на сервере. Заменить?`
      : `На сервере уже есть ${names.length} из выбранных: ${names.slice(0, 8).join(', ')}${names.length > 8 ? '…' : ''}. Заменить?`
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onNo()}>
      <div className="modal sftp-props-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Файл уже есть</h2>
        <p className="hint">{msg}</p>
        <div className="modal-actions">
          <button type="button" onClick={onNo}>
            Отмена
          </button>
          <button type="button" className="primary" onClick={onYes}>
            Заменить
          </button>
        </div>
      </div>
    </div>
  )
}

export function ExplorerHead({
  cols,
  sortCol,
  sortDir,
  onSort,
  onResize,
  onContext,
}: {
  cols: SftpColId[]
  sortCol: SftpColId
  sortDir: 'asc' | 'desc'
  onSort: (id: SftpColId) => void
  onResize: (id: SftpColId, e: React.MouseEvent) => void
  onContext: (e: React.MouseEvent) => void
}): JSX.Element {
  return (
    <div className="sftp-row sftp-row-head" onContextMenu={onContext}>
      {cols.map((id) => (
        <button
          key={id}
          type="button"
          className={'sftp-th' + (sortCol === id ? ' sorted' : '')}
          title="Сортировка · ПКМ - столбцы"
          onClick={() => onSort(id)}
        >
          <span className="sftp-th-label">{SFTP_COL_LABEL[id]}</span>
          {sortCol === id ? (
            <Icon name={sortDir === 'asc' ? 'chevron-up' : 'chevron-down'} size={12} />
          ) : null}
          <span
            className="sftp-col-resizer"
            onMouseDown={(e) => onResize(id, e)}
            onClick={(e) => e.stopPropagation()}
          />
        </button>
      ))}
    </div>
  )
}

export function CtxItem({ label, danger, onPick }: { label: string; danger?: boolean; onPick: () => void }): JSX.Element {
  return (
    <button type="button" className={'sftp-ctx-item' + (danger ? ' danger' : '')} onClick={onPick}>
      {label}
    </button>
  )
}

export function AnchoredMenu({
  x,
  y,
  className,
  children,
}: {
  x: number
  y: number
  className: string
  children: React.ReactNode
}): JSX.Element {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: x, top: y })
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const pad = 8
    const { width, height } = el.getBoundingClientRect()
    let left = x
    let top = y
    if (left + width > window.innerWidth - pad) left = window.innerWidth - pad - width
    if (left < pad) left = pad
    if (top + height > window.innerHeight - pad) top = y - height
    if (top < pad) top = pad
    if (top + height > window.innerHeight - pad) top = Math.max(pad, window.innerHeight - pad - height)
    setPos({ left, top })
  }, [x, y])
  return (
    <div
      ref={ref}
      className={className}
      style={{ left: pos.left, top: pos.top }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      {children}
    </div>
  )
}

export function PropsSheet({
  title,
  rows,
  onClose,
}: {
  title: string
  rows: { k: string; v: string }[]
  onClose: () => void
}): JSX.Element {
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal sftp-props-modal" onClick={(e) => e.stopPropagation()}>
        <h2>{title}</h2>
        <dl className="sftp-props">
          {rows.map((r) => (
            <div key={r.k} className="sftp-props-row">
              <dt>{r.k}</dt>
              <dd title={r.v}>{r.v || '—'}</dd>
            </div>
          ))}
        </dl>
        <div className="modal-actions">
          <button className="primary" onClick={onClose}>
            OK
          </button>
        </div>
      </div>
    </div>
  )
}
