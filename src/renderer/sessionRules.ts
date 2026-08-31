import type { SessionExit } from '../shared/types'
import type { PaneLeaf } from './paneTree'
import { shouldScheduleReconnect } from './reconnectFsm'

/**
 * @deprecated Используйте `shouldScheduleReconnect('session_drop', …)` из `reconnectFsm.ts`.
 */
export function shouldAutoReconnect(
  exit: Pick<SessionExit, 'reason'>,
  leaf: Pick<PaneLeaf, 'kind' | 'status'>,
  settings: { autoReconnect?: boolean }
): boolean {
  return shouldScheduleReconnect('session_drop', leaf, settings, exit)
}
