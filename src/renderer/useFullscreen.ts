import { useCallback, useEffect, useState, type RefObject } from 'react'

/**
 * Полноэкранный режим для панели рабочего стола.
 *
 * Один хук на RDP и VNC: разница между ними здесь только в том, какой элемент
 * разворачивать, а поведение обязано быть одинаковым - иначе кнопка в одной панели
 * работает не так, как в соседней.
 *
 * Выход по Esc не написан отдельно намеренно: его обрабатывает сам движок, и
 * перехватывать Esc у себя значило бы отнимать его у рабочего стола, куда он тоже нужен.
 * Состояние читаем из события, а не из своего флага: из полного экрана можно выйти
 * помимо нашей кнопки, и флаг разошёлся бы с действительностью.
 */
export function useFullscreen(ref: RefObject<HTMLElement | null>): {
  full: boolean
  toggle: () => void
} {
  const [full, setFull] = useState(false)

  useEffect(() => {
    const sync = (): void => setFull(document.fullscreenElement === ref.current)
    document.addEventListener('fullscreenchange', sync)
    sync()
    return () => document.removeEventListener('fullscreenchange', sync)
  }, [ref])

  const toggle = useCallback((): void => {
    const el = ref.current
    if (!el) return
    if (document.fullscreenElement === el) {
      void document.exitFullscreen?.()
      return
    }
    void el.requestFullscreen?.()
  }, [ref])

  return { full, toggle }
}
