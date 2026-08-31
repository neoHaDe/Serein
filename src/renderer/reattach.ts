import { emit } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { WorkspaceTool } from '../shared/types'
import { handOverSession } from './detachedSessions'
import type { PaneKind } from '../shared/types'

export interface ReattachTabPayload {
  sessionId: string
  serverId?: string
  title: string
  workspace: WorkspaceTool
  sftpOpen: boolean
  kind: PaneKind
}

export interface ReattachWorkspacePayload {
  sessionId: string
  serverId?: string
  title: string
  tool: Exclude<WorkspaceTool, 'terminal'>
}

export interface ReattachSftpPayload {
  sessionId: string
  serverId?: string
}

export async function reattachTab(payload: ReattachTabPayload): Promise<void> {
  // Отдаём сессию главному окну ДО того, как закроем своё: иначе между «мы ушли» и
  // «главное забрало» остаётся щель, в которую сессия и проваливалась.
  await handOverSession(payload.sessionId, 'main')
  await emit('serein-reattach-tab', payload)
  await invoke('windows_raise_group', { focused: 'main' }).catch(() => {})
  await getCurrentWindow().close()
}

export async function reattachWorkspace(payload: ReattachWorkspacePayload): Promise<void> {
  await emit('serein-reattach-workspace', payload)
  await invoke('windows_raise_group', { focused: 'main' }).catch(() => {})
  await getCurrentWindow().close()
}

export async function reattachSftp(payload: ReattachSftpPayload): Promise<void> {
  await emit('serein-reattach-sftp', payload)
  await invoke('windows_raise_group', { focused: 'main' }).catch(() => {})
  await getCurrentWindow().close()
}