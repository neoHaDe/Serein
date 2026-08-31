import type { SessionExit, SessionFailurePhase } from '../shared/types'
import type { PaneLeaf } from './paneTree'

/**
 * Единый автомат переподключения.
 *
 * Раньше решение «пробовать снова или показать ошибку» принималось в двух местах:
 * `handleFail` (ошибка при открытии) и обработчик `session-exit` через `shouldAutoReconnect`.
 * Условия пересекались, но не совпадали — отсюда расхождения. Все ветки сведены сюда.
 */

export type ReconnectTrigger = 'connect_fail' | 'session_drop'

const LIVE: PaneLeaf['status'][] = ['connected', 'connecting', 'reconnecting']

/** Планировать ли автопереподключение для этой панели. */
export function shouldScheduleReconnect(
  trigger: ReconnectTrigger,
  leaf: Pick<PaneLeaf, 'kind' | 'status'>,
  settings: { autoReconnect?: boolean },
  exit?: Pick<SessionExit, 'reason'>
): boolean {
  if (leaf.kind !== 'ssh') return false
  if (!settings.autoReconnect) return false

  if (trigger === 'connect_fail') {
    return leaf.status === 'connecting' || leaf.status === 'reconnecting'
  }

  if (exit?.reason !== 'drop') return false
  return LIVE.includes(leaf.status)
}

const PHASE_LABEL: Record<SessionFailurePhase, string> = {
  connect: 'при подключении',
  auth: 'при авторизации',
  jump: 'на jump-хосте',
  shell: 'во время сессии',
  hostkey: 'при проверке ключа хоста'
}

/** Сообщение для панели, когда переподключение не планируется. */
export function sessionClosedMessage(
  exit: Pick<SessionExit, 'error' | 'phase'>,
  fallback = 'Сессия завершена'
): string {
  const base = exit.error?.trim() || fallback
  const phase = exit.phase ? PHASE_LABEL[exit.phase] : undefined
  return phase ? `${base} (${phase})` : base
}
