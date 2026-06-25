/**
 * Мост renderer ↔ Rust-бэкенд для Tauri-сборки TermiNAL.
 * Воспроизводит ровно тот же интерфейс, что Electron-preload (`window.api`),
 * поэтому весь renderer переносится без правок. Реальные команды идут через
 * Tauri `invoke`/`listen`; неперенесённые модули пока возвращают заглушки.
 */
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog'
import type {
  ServerConfig,
  OpenSshPayload,
  OpenLocalPayload,
  ResizePayload,
  SessionData,
  SessionExit,
  SessionStatus,
  SftpListResult,
  TransferProgress,
  TransferItem,
  AppSettings,
  TunnelStatus,
  Snippet,
  KIPrompt,
  GenerateKeyParams,
  GeneratedKey,
  LocalListResult,
  RemoteEditStatus,
  RemoteFileContent,
  WriteFileResult,
  SerializedTab,
  ServerMetrics,
  DockerListResult,
  DockerAction
} from '../shared/types'

/** Подписка на событие Tauri с синхронной функцией отписки (как в Electron-preload). */
function sub<T>(event: string, cb: (payload: T) => void): () => void {
  const un = listen<T>(event, (e) => cb(e.payload))
  return () => {
    void un.then((f) => f())
  }
}

/** Ошибка для ещё не перенесённых на Rust возможностей. */
function notYet(feature: string): Promise<never> {
  return Promise.reject(new Error(`«${feature}» ещё не перенесён в Tauri-сборку`))
}

export const api = {
  settings: {
    get: (): Promise<AppSettings> => invoke('settings_get'),
    set: (patch: Partial<AppSettings>): Promise<AppSettings> => invoke('settings_set', { patch })
  },
  servers: {
    list: (): Promise<ServerConfig[]> => invoke('servers_list'),
    save: (cfg: ServerConfig): Promise<ServerConfig> => invoke('servers_save', { cfg }),
    remove: (id: string): Promise<void> => invoke('servers_delete', { id }),
    importSshConfig: (): Promise<{ imported: number }> => invoke('servers_import_ssh_config'),
    importPutty: (): Promise<{ imported: number }> => invoke('servers_import_putty')
  },
  session: {
    openSsh: (p: OpenSshPayload): Promise<string> => invoke('session_open_ssh', { p }),
    openLocal: (p: OpenLocalPayload): Promise<string> => invoke('session_open_local', { p }),
    ping: (id: string): Promise<number | null> => invoke('session_ping', { id }),
    monitor: (id: string): Promise<ServerMetrics> => invoke('session_monitor', { id }),
    logStatus: (_id: string): Promise<boolean> => Promise.resolve(false),
    logToggle: (_id: string, _title: string): Promise<{ logging: boolean; path?: string }> =>
      notYet('Логирование сессии'),
    write: (id: string, data: string): void => void invoke('session_write', { id, data }),
    resize: (p: ResizePayload): void => void invoke('session_resize', { p }),
    close: (id: string): Promise<void> => invoke('session_close', { id }),
    onData: (cb: (p: SessionData) => void) => sub<SessionData>('session-data', cb),
    onExit: (cb: (p: SessionExit) => void) => sub<SessionExit>('session-exit', cb),
    onStatus: (cb: (p: SessionStatus) => void) => sub<SessionStatus>('session-status', cb),
    onKi: (cb: (p: { id: string; prompts: KIPrompt[] }) => void) =>
      sub<{ id: string; prompts: KIPrompt[] }>('session-ki', cb),
    respondKi: (id: string, answers: string[]): Promise<void> =>
      invoke('session_ki_respond', { id, answers })
  },
  sftp: {
    list: (sessionId: string, path: string): Promise<SftpListResult> =>
      invoke('sftp_list', { sessionId, path }),
    upload: async (sessionId: string, remoteDir: string): Promise<{ uploaded: number }> => {
      const sel = await openDialog({ multiple: true, directory: false, title: 'Файлы для загрузки на сервер' })
      const paths = Array.isArray(sel) ? sel : sel ? [sel] : []
      if (!paths.length) return { uploaded: 0 }
      return invoke('sftp_upload_paths', { sessionId, remoteDir, paths })
    },
    download: async (sessionId: string, remotePath: string): Promise<{ saved: boolean; path?: string }> => {
      const dir = await openDialog({ directory: true, title: 'Куда сохранить' })
      if (typeof dir !== 'string') return { saved: false }
      await invoke('sftp_download_to', { sessionId, remotePath, localDir: dir })
      return { saved: true, path: dir }
    },
    mkdir: (sessionId: string, path: string): Promise<void> => invoke('sftp_mkdir', { sessionId, path }),
    remove: (sessionId: string, path: string, isDir: boolean): Promise<void> =>
      invoke('sftp_remove', { sessionId, path, isDir }),
    rename: (sessionId: string, from: string, to: string): Promise<void> =>
      invoke('sftp_rename', { sessionId, from, to }),
    uploadFolder: async (sessionId: string, remoteDir: string): Promise<{ uploaded: number }> => {
      const sel = await openDialog({ directory: true, title: 'Папка для загрузки на сервер' })
      if (typeof sel !== 'string') return { uploaded: 0 }
      return invoke('sftp_upload_paths', { sessionId, remoteDir, paths: [sel] })
    },
    uploadPaths: (sessionId: string, remoteDir: string, paths: string[]): Promise<{ uploaded: number }> =>
      invoke('sftp_upload_paths', { sessionId, remoteDir, paths }),
    downloadTo: (sessionId: string, remotePath: string, localDir: string): Promise<void> =>
      invoke('sftp_download_to', { sessionId, remotePath, localDir }),
    listTransfers: (): Promise<TransferItem[]> => Promise.resolve([]),
    cancelTransfer: (_id: string): Promise<void> => Promise.resolve(),
    clearFinished: (): Promise<TransferItem[]> => Promise.resolve([]),
    readFile: (sessionId: string, remotePath: string): Promise<RemoteFileContent> =>
      invoke('sftp_read_file', { sessionId, remotePath }),
    writeFile: (
      sessionId: string,
      remotePath: string,
      content: string,
      mode: number,
      baseMtime: number,
      eol: 'lf' | 'crlf'
    ): Promise<WriteFileResult> =>
      invoke('sftp_write_file', { sessionId, remotePath, content, mode, baseMtime, eol }),
    edit: (sessionId: string, remotePath: string): Promise<void> => invoke('sftp_edit', { sessionId, remotePath }),
    editStop: (sessionId: string, remotePath: string): Promise<void> =>
      invoke('sftp_edit_stop', { sessionId, remotePath }),
    startDrag: (_s: string, _r: string): void => {},
    onProgress: (cb: (p: TransferProgress) => void) => sub<TransferProgress>('sftp-progress', cb),
    onTransfer: (cb: (p: TransferItem) => void) => sub<TransferItem>('sftp-transfer', cb),
    onEditStatus: (cb: (p: RemoteEditStatus) => void) => sub<RemoteEditStatus>('sftp-edit-status', cb)
  },
  localfs: {
    list: (path: string): Promise<LocalListResult> => invoke('localfs_list', { path }),
    home: (): Promise<string> => invoke('localfs_home'),
    parent: (path: string): Promise<string> => invoke('localfs_parent', { path })
  },
  dialog: {
    pickKey: async (): Promise<string | null> => {
      const res = await openDialog({ multiple: false, directory: false, title: 'Выберите приватный SSH-ключ' })
      return typeof res === 'string' ? res : null
    }
  },
  files: {
    // В Tauri путь перетащенного файла приходит через onDragDrop окна; здесь заглушка.
    pathForFile: (_file: File): string => ''
  },
  layout: {
    get: (): Promise<SerializedTab[]> => invoke('layout_get'),
    set: (tabs: SerializedTab[]): Promise<void> => invoke('layout_set', { tabs })
  },
  docker: {
    list: (id: string): Promise<DockerListResult> => invoke('docker_list', { id }),
    action: (id: string, containerId: string, action: DockerAction): Promise<{ ok: boolean; error?: string }> =>
      invoke('docker_action', { id, containerId, action }),
    logs: (id: string, containerId: string): Promise<{ ok: boolean; logs?: string; error?: string }> =>
      invoke('docker_logs', { id, containerId })
  },
  vault: {
    status: (): Promise<{ enabled: boolean; locked: boolean }> => invoke('vault_status'),
    unlock: (password: string): Promise<boolean> => invoke('vault_unlock', { password }),
    enable: (password: string): Promise<{ ok: boolean; error?: string }> => invoke('vault_enable', { password }),
    disable: (password: string): Promise<{ ok: boolean; error?: string }> => invoke('vault_disable', { password })
  },
  backup: {
    export: async (password: string): Promise<{ saved: boolean; path?: string }> => {
      const path = await saveDialog({
        title: 'Сохранить бэкап',
        defaultPath: `terminal-backup-${new Date().toISOString().slice(0, 10)}.tbk`,
        filters: [{ name: 'TermiNAL backup', extensions: ['tbk'] }]
      })
      if (!path) return { saved: false }
      return invoke('backup_export', { password, path })
    },
    import: async (password: string): Promise<{ imported: boolean; servers?: number; snippets?: number }> => {
      const sel = await openDialog({ title: 'Файл бэкапа', filters: [{ name: 'TermiNAL backup', extensions: ['tbk'] }] })
      if (typeof sel !== 'string') return { imported: false }
      return invoke('backup_import', { password, path: sel })
    }
  },
  snippets: {
    list: (): Promise<Snippet[]> => invoke('snippets_list'),
    save: (s: Snippet): Promise<Snippet> => invoke('snippets_save', { s }),
    remove: (id: string): Promise<void> => invoke('snippets_delete', { id })
  },
  keygen: {
    generate: (params: GenerateKeyParams): Promise<GeneratedKey> => invoke('keygen_generate', { params }),
    save: async (
      key: GeneratedKey,
      defaultName: string
    ): Promise<{ saved: boolean; privatePath?: string; publicPath?: string }> => {
      const path = await saveDialog({ title: 'Сохранить приватный ключ', defaultPath: defaultName })
      if (!path) return { saved: false }
      return invoke('keygen_save', { path, key })
    },
    install: (sessionId: string, publicKey: string): Promise<{ installed: boolean }> =>
      invoke('keygen_install', { sessionId, publicKey })
  },
  tunnel: {
    listStatus: (sessionId: string): Promise<TunnelStatus[]> => invoke('tunnel_list_status', { sessionId }),
    open: (sessionId: string, tunnelId: string): Promise<void> => invoke('tunnel_open', { sessionId, tunnelId }),
    close: (sessionId: string, tunnelId: string): Promise<void> => invoke('tunnel_close', { sessionId, tunnelId }),
    onStatus: (cb: (s: TunnelStatus) => void) => sub<TunnelStatus>('tunnel-status', cb)
  }
}

export type Api = typeof api
