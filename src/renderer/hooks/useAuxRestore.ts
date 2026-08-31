import { useCallback, useRef } from 'react'
import type { SavedAuxWindow } from '../../shared/types'
import { openAuxWindow, sanitizeWindowLabel } from '../auxWindows'
import { auxWindowKey } from '../auxLayout'
import { openDetachedLogsWindow } from '../components/dockerLogs'

/**
 * Восстановление дополнительных окон (SFTP, логи контейнера) после перезапуска.
 *
 * Тонкость, ради которой это отдельный хук: окно нельзя открыть сразу при старте. Оно
 * привязано к SSH-сессии, а сессии на момент чтения раскладки ещё нет — она появится,
 * когда терминал подключится. Поэтому сохранённые окна лежат «в ожидании», и каждое
 * открывается в тот момент, когда поднялась сессия к его серверу.
 *
 * Отсюда и второй набор — уже открытых. Без него окно открывалось бы повторно на каждое
 * новое подключение к тому же серверу: переподключился по обрыву — и получил второй SFTP.
 */
export interface AuxRestore {
  /** Запомнить сохранённые окна. Зовётся один раз, когда прочитана раскладка. */
  remember: (windows: SavedAuxWindow[]) => void
  /** Сессия к серверу поднялась — открыть всё, что её ждало. */
  tryRestoreFor: (serverId: string, sessionId: string) => void
}

export function useAuxRestore(): AuxRestore {
  const pending = useRef<SavedAuxWindow[]>([])
  const opened = useRef(new Set<string>())

  const remember = useCallback((windows: SavedAuxWindow[]) => {
    pending.current = windows
  }, [])

  const open = useCallback(async (w: SavedAuxWindow, sessionId: string) => {
    try {
      if (w.kind === 'sftp') {
        await openAuxWindow({
          label: 'sftp-' + sanitizeWindowLabel(sessionId),
          query: { sftp: '1', sessionId },
          title: 'SFTP',
          width: Math.max(w.w || 480, 420),
          height: Math.max(w.h || 720, 280),
          x: w.x,
          y: w.y,
          persist: { kind: 'sftp', serverId: w.serverId }
        })
      } else if (w.containerId) {
        await openDetachedLogsWindow({
          sessionId,
          serverId: w.serverId,
          containerId: w.containerId,
          name: w.name ?? w.containerId,
          width: Math.max(w.w || 800, 420),
          height: Math.max(w.h || 560, 280),
          x: w.x,
          y: w.y
        })
      }
    } catch {
      // Не открылось — забываем отметку, чтобы следующая попытка не считалась дублем.
      opened.current.delete(auxWindowKey(w))
    }
  }, [])

  const tryRestoreFor = useCallback(
    (serverId: string, sessionId: string) => {
      for (const w of pending.current) {
        if (w.serverId !== serverId) continue
        const key = auxWindowKey(w)
        if (opened.current.has(key)) continue
        opened.current.add(key)
        void open(w, sessionId)
      }
    },
    [open]
  )

  return { remember, tryRestoreFor }
}
