import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'

export interface MenuItem {
  label: string
  onClick: () => void
  danger?: boolean
  /** Разделитель рисуется над пунктом. */
  separated?: boolean
}

interface Props {
  x: number
  y: number
  items: MenuItem[]
  onClose: () => void
}

/**
 * Меню по правому клику. Открывается в точке курсора и **прижимается к экрану**:
 * у нижнего края списка меню иначе уезжает за пределы окна и часть пунктов недоступна.
 */
export function ContextMenu({ x, y, items, onClose }: Props): JSX.Element {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: x, top: y })

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const { width, height } = el.getBoundingClientRect()
    const pad = 8
    setPos({
      left: Math.min(x, window.innerWidth - width - pad),
      top: Math.min(y, window.innerHeight - height - pad)
    })
  }, [x, y])

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  // Через портал в body: сайдбар — отдельный слой (`position: relative; z-index`),
  // и меню внутри него перекрывалось соседним разделителем панели.
  return createPortal(
    <>
      <div
        className="ctx-backdrop"
        onMouseDown={onClose}
        onContextMenu={(e) => {
          // Правый клик мимо меню закрывает его, а не открывает второе поверх.
          e.preventDefault()
          onClose()
        }}
      />
      <div ref={ref} className="ctx-menu" style={{ left: pos.left, top: pos.top }}>
        {items.map((it, i) => (
          <button
            key={i}
            className={'ctx-item' + (it.danger ? ' danger' : '') + (it.separated ? ' separated' : '')}
            onClick={() => {
              onClose()
              it.onClick()
            }}
          >
            {it.label}
          </button>
        ))}
      </div>
    </>,
    document.body
  )
}
