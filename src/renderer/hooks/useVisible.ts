import { useEffect, useRef, useState } from 'react'

/**
 * Смотрит ли кто-нибудь на этот кусок интерфейса прямо сейчас.
 *
 * Два условия сразу, и оба нужны. Первое - окно не свёрнуто и вкладка движка активна
 * (`document.hidden`). Второе - сам элемент на экране, а не спрятан под `display: none`:
 * вкладки приложения не размонтируются при переключении, они прячутся, и обзор фоновой
 * вкладки иначе продолжает опрашивать сервер, хотя показать результат некому.
 *
 * Возвращает ссылку, которую надо повесить на корневой узел, и признак видимости.
 */
export function useVisible<T extends HTMLElement>(): [React.RefObject<T>, boolean] {
  const ref = useRef<T>(null)
  const [visible, setVisible] = useState(true)

  useEffect(() => {
    const node = ref.current
    if (!node) return

    const compute = (): void => {
      // offsetParent пуст у всего, что скрыто через display: none, и это дешевле,
      // чем getComputedStyle на каждый тик.
      const onScreen = node.offsetParent !== null
      setVisible(onScreen && !document.hidden)
    }

    compute()
    document.addEventListener('visibilitychange', compute)
    // Показ и скрытие вкладки меняют размер узла с нулевого на настоящий, поэтому
    // наблюдателя размера хватает: отдельный опрос по таймеру не нужен.
    const ro = new ResizeObserver(compute)
    ro.observe(node)

    return () => {
      document.removeEventListener('visibilitychange', compute)
      ro.disconnect()
    }
  }, [])

  return [ref, visible]
}
