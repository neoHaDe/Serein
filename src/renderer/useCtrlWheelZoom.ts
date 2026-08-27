import { useEffect, useState } from 'react'

function readZoom(key: string, fallback: number): number {
  const n = Number(localStorage.getItem(key))
  return Number.isFinite(n) && n >= 0.7 && n <= 2.2 ? n : fallback
}

function clampZoom(z: number): number {
  return Math.min(2.2, Math.max(0.7, Math.round(z * 10) / 10))
}

/** Ctrl/Cmd + колесо, +/-, 0 — как в браузере. Вешается на контейнер. */
export function useCtrlWheelZoom(storageKey: string, fallback = 1): {
  zoom: number
  ref: (el: HTMLElement | null) => void
  reset: () => void
} {
  const [el, setEl] = useState<HTMLElement | null>(null)
  const [zoom, setZoom] = useState(() => readZoom(storageKey, fallback))

  useEffect(() => {
    if (!el) return
    const bump = (delta: number): void => {
      setZoom((z) => {
        const n = clampZoom(delta === 0 ? 1 : z + delta)
        localStorage.setItem(storageKey, String(n))
        return n
      })
    }
    const over = (t: EventTarget | null): boolean => t instanceof Node && el.contains(t)
    const onWheel = (e: WheelEvent): void => {
      if (!e.ctrlKey && !e.metaKey) return
      if (!over(e.target) && !el.matches(':hover')) return
      e.preventDefault()
      bump(e.deltaY < 0 ? 0.1 : -0.1)
    }
    const onKey = (e: KeyboardEvent): void => {
      if (!e.ctrlKey && !e.metaKey) return
      const hover = el.matches(':hover')
      const focus = document.activeElement && el.contains(document.activeElement)
      if (!hover && !focus) return
      if (e.key === '=' || e.key === '+') {
        e.preventDefault()
        bump(0.1)
      } else if (e.key === '-' || e.key === '_') {
        e.preventDefault()
        bump(-0.1)
      } else if (e.key === '0') {
        e.preventDefault()
        bump(0)
      }
    }
    window.addEventListener('wheel', onWheel, { passive: false, capture: true })
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('wheel', onWheel, true)
      window.removeEventListener('keydown', onKey)
    }
  }, [el, storageKey])

  return {
    zoom,
    ref: setEl,
    reset: () => {
      setZoom(1)
      localStorage.setItem(storageKey, '1')
    }
  }
}
