import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Icon } from './Icon'

function minimizedLabel(n: number): string {
  if (n === 1) return '1 окно свёрнуто'
  if (n >= 2 && n <= 4) return `${n} окна свёрнуто`
  return `${n} окон свёрнуто`
}

/** Кнопка в main, когда откреплённые окна свёрнуты без иконки на панели задач. */
export function MinimizedWindowsHint(): JSX.Element | null {
  const [count, setCount] = useState(0)

  useEffect(() => {
    let alive = true
    let stopFocus: (() => void) | undefined
    let intervalId: number | undefined

    void (async () => {
      try {
        const me = getCurrentWindow()
        if (me.label !== 'main') return

        const poll = async (): Promise<void> => {
          try {
            const n = await invoke<number>('windows_count_minimized')
            if (alive) setCount(n)
          } catch {
            /* */
          }
        }

        await poll()
        intervalId = window.setInterval(() => {
          void poll()
        }, 1500)
        stopFocus = await me.onFocusChanged((e) => {
          if (e.payload) void poll()
        })
      } catch {
        /* не Tauri */
      }
    })()

    return () => {
      alive = false
      if (intervalId !== undefined) window.clearInterval(intervalId)
      stopFocus?.()
    }
  }, [])

  if (!count) return null

  return (
    <button
      type="button"
      className="sb-item sb-minimized-hint"
      title="Развернуть свёрнутые откреплённые окна"
      onClick={() => {
        void invoke('windows_restore_minimized').then(() => setCount(0))
      }}
    >
      <Icon name="win-restore" size={12} /> {minimizedLabel(count)} — развернуть
    </button>
  )
}