import { useCallback, useEffect, useRef } from 'react'
import type { PaneLeaf } from '../paneTree'

/**
 * Переподключение панели после обрыва: расписание попыток, ручной повтор, отмена.
 *
 * Держалось на двух `Map` в рефах посреди `App.tsx` вперемешку со всем остальным. Логика
 * тут своя и замкнутая — таймеры, счётчик попыток, задержка с удвоением, — а наружу нужен
 * ровно один способ поменять состояние панели. Его и просим параметром вместо того, чтобы
 * тащить сюда `setTabs` со всей формой вкладок: хук не должен знать, что панели живут в
 * дереве, а дерево — во вкладках.
 */

/** Сколько раз пробуем переподключиться сами, прежде чем оставить это пользователю. */
export const RECONNECT_MAX = 5

export interface ReconnectDeps {
  /** Поменять состояние конкретной панели. Патч либо готовый, либо от прежнего значения. */
  patchPane: (
    tabKey: string,
    paneId: string,
    patch: Partial<PaneLeaf> | ((leaf: PaneLeaf) => Partial<PaneLeaf>)
  ) => void
}

export interface Reconnect {
  /** Забыть таймер и счётчик: панель закрыли или она подключилась. */
  clear: (paneId: string) => void
  /** Запланировать следующую попытку с растущей задержкой. */
  schedule: (tabKey: string, paneId: string) => void
  /** Повтор по кнопке: счётчик сбрасывается, ждать не надо. */
  now: (tabKey: string, paneId: string) => void
  /** Отмена: больше не пробуем. */
  cancel: (tabKey: string, paneId: string) => void
}

export function useReconnect({ patchPane }: ReconnectDeps): Reconnect {
  const attempts = useRef(new Map<string, number>())
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>())

  // При размонтировании гасим все таймеры: иначе отложенная попытка сработает уже после
  // того, как окна не стало, и полезет в состояние, которого нет.
  useEffect(() => {
    const running = timers.current
    return () => {
      for (const t of running.values()) clearTimeout(t)
      running.clear()
    }
  }, [])

  const stopTimer = (paneId: string): void => {
    const t = timers.current.get(paneId)
    if (t) {
      clearTimeout(t)
      timers.current.delete(paneId)
    }
  }

  const clear = useCallback((paneId: string) => {
    stopTimer(paneId)
    attempts.current.delete(paneId)
  }, [])

  /** Поднять панель заново: новое поколение заставляет терминал пересоздать сессию. */
  const restart = useCallback(
    (tabKey: string, paneId: string) => {
      patchPane(tabKey, paneId, (l) => ({
        gen: l.gen + 1,
        status: 'connecting',
        statusMsg: undefined,
        sessionId: undefined
      }))
    },
    [patchPane]
  )

  const schedule = useCallback(
    (tabKey: string, paneId: string) => {
      stopTimer(paneId)
      const n = (attempts.current.get(paneId) ?? 0) + 1
      if (n > RECONNECT_MAX) {
        attempts.current.delete(paneId)
        patchPane(tabKey, paneId, {
          status: 'error',
          statusMsg: `Не удалось переподключить после ${RECONNECT_MAX} попыток`,
          sessionId: undefined
        })
        return
      }
      attempts.current.set(paneId, n)
      // Удвоение с потолком в 15 секунд: сервер после перезагрузки поднимается не сразу,
      // а долбиться в него раз в секунду бессмысленно и заметно в его логах.
      const delay = Math.min(15_000, 1000 * 2 ** (n - 1))
      patchPane(tabKey, paneId, {
        status: 'reconnecting',
        statusMsg: `Переподключение… попытка ${n}/${RECONNECT_MAX}`,
        sessionId: undefined
      })
      timers.current.set(
        paneId,
        setTimeout(() => {
          timers.current.delete(paneId)
          restart(tabKey, paneId)
        }, delay)
      )
    },
    [patchPane, restart]
  )

  const now = useCallback(
    (tabKey: string, paneId: string) => {
      clear(paneId)
      restart(tabKey, paneId)
    },
    [clear, restart]
  )

  const cancel = useCallback(
    (tabKey: string, paneId: string) => {
      clear(paneId)
      patchPane(tabKey, paneId, {
        status: 'closed',
        statusMsg: 'Переподключение отменено',
        sessionId: undefined
      })
    },
    [clear, patchPane]
  )

  return { clear, schedule, now, cancel }
}
