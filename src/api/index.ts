/**
 * Мост renderer ↔ Rust: `window.api` через Tauri `invoke` / `listen`.
 * Неперенесённые модули пока возвращают заглушки.
 */
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog'
import type {
  ServerConfig,
  MultiExecResult,
  AgentIdentitiesResult,
  HostKeyRequest,
  KnownHostEntry,
  SerialConfig,
  SerialPortInfo,
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
  SftpPreview,
  WriteFileResult,
  SerializedTab,
  AuxLayout,
  ServerMetrics,
  DockerListResult,
  DockerAction,
  DockerStatsResult,
  DockerLogsChunk,
  DockerContainerFilesResult,
  DockerComposeListResult,
  DockerComposePsResult,
  DockerComposeAction,
  WorkspaceProcess,
  WorkspaceService
} from '../shared/types'

/** Подписка на событие Tauri с синхронной функцией отписки (как в Electron-preload). */
function sub<T>(event: string, cb: (payload: T) => void): () => void {
  const un = listen<T>(event, (e) => cb(e.payload))
  return () => {
    void un.then((f) => f())
  }
}

export const api = {
  clipboard: {
    write: (text: string): Promise<void> => invoke('clipboard_write', { text }),
    read: (): Promise<string> => invoke('clipboard_read')
  },
  settings: {
    get: (): Promise<AppSettings> => invoke('settings_get'),
    set: (patch: Partial<AppSettings>): Promise<AppSettings> => invoke('settings_set', { patch })
  },
  servers: {
    list: (): Promise<ServerConfig[]> => invoke('servers_list'),
    save: (cfg: ServerConfig): Promise<ServerConfig> => invoke('servers_save', { cfg }),
    remove: (id: string): Promise<void> => invoke('servers_delete', { id }),
    /** Перестановка после перетаскивания: меняет только группу и позицию. */
    reorder: (items: { id: string; group: string; order: number }[]): Promise<void> =>
      invoke('servers_reorder', { items }),
    importSshConfig: (): Promise<{ imported: number }> => invoke('servers_import_ssh_config'),
    importPutty: (): Promise<{ imported: number }> => invoke('servers_import_putty')
  },
  agent: {
    /** Ключи локального SSH-агента. `ok: false` — агент не запущен, не ошибка вызова. */
    identities: (): Promise<AgentIdentitiesResult> => invoke('ssh_agent_identities')
  },
  knownHosts: {
    list: (): Promise<KnownHostEntry[]> => invoke('knownhosts_list'),
    forget: (host: string): Promise<boolean> => invoke('knownhosts_forget', { host }),
    importOpenssh: (): Promise<{ imported: number }> => invoke('knownhosts_import')
  },
  serial: {
    ports: (): Promise<SerialPortInfo[]> => invoke('serial_ports'),
    /** BREAK на линию (recovery-режим сетевого железа). */
    sendBreak: (id: string): Promise<void> => invoke('serial_send_break', { id }),
    setSignal: (id: string, line: 'dtr' | 'rts', on: boolean): Promise<void> =>
      invoke('serial_set_signal', { id, line, on })
  },
  telnet: {
    /**
     * Управляющая команда telnet. `interrupt` — то же, что Ctrl+C на настоящем терминале,
     * но проходит даже когда железка перестала читать поток данных.
     */
    command: (
      id: string,
      name: 'break' | 'interrupt' | 'abort-output' | 'are-you-there' | 'erase-char' | 'erase-line'
    ): Promise<void> => invoke('telnet_command', { id, name })
  },
  session: {
    openSsh: (p: OpenSshPayload): Promise<string> => invoke('session_open_ssh', { p }),
    openLocal: (p: OpenLocalPayload): Promise<string> => invoke('session_open_local', { p }),
    /** COM-порт: по профилю сервера (`serverId`) либо разовыми настройками (`serial`). */
    openSerial: (p: { serverId?: string; serial?: SerialConfig }): Promise<string> =>
      invoke('session_open_serial', { p }),
    /** Telnet или «сырой» TCP: по профилю (`serverId`) либо разовыми параметрами. */
    openTcp: (p: {
      serverId?: string
      connection?: 'telnet' | 'raw'
      host?: string
      port?: number
      cols?: number
      rows?: number
    }): Promise<string> => invoke('session_open_tcp', { p }),
    ping: (id: string): Promise<number | null> => invoke('session_ping', { id }),
    monitor: (id: string): Promise<ServerMetrics> => invoke('session_monitor', { id }),
    logStatus: (id: string): Promise<boolean> => invoke('session_log_status', { id }),
    logToggle: (id: string, title: string): Promise<{ logging: boolean; path?: string }> =>
      invoke('session_log_toggle', { id, title }),
    write: (id: string, data: string): void => void invoke('session_write', { id, data }),
    resize: (p: ResizePayload): void => void invoke('session_resize', { p }),
    close: (id: string): Promise<void> => invoke('session_close', { id }),
    onData: (cb: (p: SessionData) => void) => sub<SessionData>('session-data', cb),
    onExit: (cb: (p: SessionExit) => void) => sub<SessionExit>('session-exit', cb),
    onStatus: (cb: (p: SessionStatus) => void) => sub<SessionStatus>('session-status', cb),
    onKi: (cb: (p: { id: string; prompts: KIPrompt[] }) => void) =>
      sub<{ id: string; prompts: KIPrompt[] }>('session-ki', cb),
    /** Сервер предъявил незнакомый или изменившийся ключ — ждём решения пользователя. */
    onHostKey: (cb: (p: HostKeyRequest) => void) => sub<HostKeyRequest>('session-hostkey', cb),
    respondHostKey: (requestId: string, accept: boolean): Promise<void> =>
      invoke('session_hostkey_respond', { requestId, accept }),
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
    chmod: (sessionId: string, path: string, mode: number): Promise<void> =>
      invoke('sftp_chmod', { sessionId, path, mode }),
    preview: (sessionId: string, remotePath: string): Promise<SftpPreview> =>
      invoke('sftp_preview', { sessionId, remotePath }),
    uploadFolder: async (sessionId: string, remoteDir: string): Promise<{ uploaded: number }> => {
      const sel = await openDialog({ directory: true, title: 'Папка для загрузки на сервер' })
      if (typeof sel !== 'string') return { uploaded: 0 }
      return invoke('sftp_upload_paths', { sessionId, remoteDir, paths: [sel] })
    },
    uploadPaths: (sessionId: string, remoteDir: string, paths: string[]): Promise<{ uploaded: number }> =>
      invoke('sftp_upload_paths', { sessionId, remoteDir, paths }),
    nameConflicts: (sessionId: string, remoteDir: string, names: string[]): Promise<string[]> =>
      invoke('sftp_name_conflicts', { sessionId, remoteDir, names }),
    downloadTo: (sessionId: string, remotePath: string, localDir: string): Promise<void> =>
      invoke('sftp_download_to', { sessionId, remotePath, localDir }),
    startOsDrag: (sessionId: string, remotePaths: string[]): Promise<void> =>
      invoke('sftp_drag_out', { sessionId, remotePaths }),
    cancelTransfer: (id: string): Promise<void> => invoke('sftp_cancel_transfer', { id }),
    pauseTransfer: (id: string): Promise<void> => invoke('sftp_pause_transfer', { id }),
    resumeTransfer: (id: string): Promise<void> => invoke('sftp_resume_transfer', { id }),
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
    onProgress: (cb: (p: TransferProgress) => void) => sub<TransferProgress>('sftp-progress', cb),
    onTransfer: (cb: (p: TransferItem) => void) => sub<TransferItem>('sftp-transfer', cb),
    onEditStatus: (cb: (p: RemoteEditStatus) => void) => sub<RemoteEditStatus>('sftp-edit-status', cb)
  },
  localfs: {
    list: (path: string): Promise<LocalListResult> => invoke('localfs_list', { path }),
    home: (): Promise<string> => invoke('localfs_home'),
    parent: (path: string): Promise<string> => invoke('localfs_parent', { path }),
    copyInto: (paths: string[], destDir: string): Promise<number> =>
      invoke('localfs_copy_into', { paths, destDir })
  },
  dialog: {
    pickKey: async (): Promise<string | null> => {
      const res = await openDialog({ multiple: false, directory: false, title: 'Выберите приватный SSH-ключ' })
      return typeof res === 'string' ? res : null
    }
  },
  files: {
    pick: async (opts: { title: string; multiple?: boolean; directory?: boolean }): Promise<string[]> => {
      const sel = await openDialog({
        title: opts.title,
        multiple: opts.directory ? false : opts.multiple !== false,
        directory: !!opts.directory
      })
      if (Array.isArray(sel)) return sel
      return typeof sel === 'string' ? [sel] : []
    }
  },
  layout: {
    get: (): Promise<SerializedTab[]> => invoke('layout_get'),
    set: (tabs: SerializedTab[]): Promise<void> => invoke('layout_set', { tabs })
  },
  auxLayout: {
    get: (): Promise<AuxLayout> => invoke('aux_layout_get'),
    set: (layout: AuxLayout): Promise<void> => invoke('aux_layout_set', { layout })
  },
  docker: {
    list: (id: string): Promise<DockerListResult> => invoke('docker_list', { id }),
    action: (id: string, containerId: string, action: DockerAction): Promise<{ ok: boolean; error?: string }> =>
      invoke('docker_action', { id, containerId, action }),
    stats: (id: string, containerId: string): Promise<DockerStatsResult> =>
      invoke('docker_stats', { id, containerId }),
    logs: (id: string, containerId: string): Promise<{ ok: boolean; logs?: string; error?: string }> =>
      invoke('docker_logs', { id, containerId }),
    cancelLogs: (id: string, containerId?: string): Promise<void> =>
      invoke('docker_logs_cancel', { id, containerId: containerId ?? null }),
    onLogs: (cb: (p: DockerLogsChunk) => void) => sub<DockerLogsChunk>('docker-logs', cb),
    files: (id: string, containerId: string, path: string): Promise<DockerContainerFilesResult> =>
      invoke('docker_container_files', { id, containerId, path }),
    composeList: (id: string): Promise<DockerComposeListResult> => invoke('docker_compose_list', { id }),
    composePs: (id: string, composeFile: string, project: string): Promise<DockerComposePsResult> =>
      invoke('docker_compose_ps', { id, composeFile, project }),
    composeAction: (
      id: string,
      composeFile: string,
      project: string,
      action: DockerComposeAction,
      service?: string
    ): Promise<{ ok: boolean; error?: string }> =>
      invoke('docker_compose_action', { id, composeFile, project, action, service: service ?? null }),
    composeRead: (id: string, composeFile: string): Promise<{ ok: boolean; text?: string; error?: string }> =>
      invoke('docker_compose_read', { id, composeFile }),
    composeLogs: (
      id: string,
      composeFile: string,
      project: string,
      service: string
    ): Promise<{ ok: boolean; logs?: string; error?: string }> =>
      invoke('docker_compose_logs', { id, composeFile, project, service }),
    cancelComposeLogs: (id: string, composeFile?: string, service?: string): Promise<void> =>
      invoke('docker_compose_logs_cancel', { id, composeFile: composeFile ?? null, service: service ?? null })
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
        filters: [{ name: 'Serein backup', extensions: ['tbk'] }]
      })
      if (!path) return { saved: false }
      return invoke('backup_export', { password, path })
    },
    import: async (
      password: string
    ): Promise<{
      imported: boolean
      servers?: number
      snippets?: number
      /** Скольким серверам подставили путь к ключу под текущую систему. */
      keysRemapped?: number
      /** Профили с командой-посредником: она запускается на этой машине при подключении. */
      proxyCommands?: { name: string; command: string }[]
    }> => {
      const sel = await openDialog({ title: 'Файл бэкапа', filters: [{ name: 'Serein backup', extensions: ['tbk'] }] })
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
  },
  workspace: {
    processes: (sessionId: string): Promise<{ ok: boolean; error?: string; rows?: WorkspaceProcess[] }> =>
      invoke('workspace_processes', { sessionId }),
    kill: (sessionId: string, pid: number): Promise<{ ok: boolean; error?: string }> =>
      invoke('workspace_kill', { sessionId, pid }),
    services: (sessionId: string): Promise<{ ok: boolean; error?: string; rows?: WorkspaceService[] }> =>
      invoke('workspace_services', { sessionId }),
    serviceAction: (
      sessionId: string,
      name: string,
      action: 'start' | 'stop' | 'restart'
    ): Promise<{ ok: boolean; error?: string }> =>
      invoke('workspace_service_action', { sessionId, name, action }),
    logs: (sessionId: string): Promise<{ ok: boolean; error?: string; text?: string }> =>
      invoke('workspace_logs', { sessionId })
  },
  multi: {
    /**
     * Одна команда на нескольких серверах. Полный список возвращается в конце,
     * но результат каждого хоста приходит событием сразу — ждать самый медленный,
     * чтобы увидеть первый, незачем.
     */
    exec: (serverIds: string[], command: string): Promise<MultiExecResult[]> =>
      invoke('multi_exec', { serverIds, command }),
    onResult: (cb: (p: { done: number; total: number; result: MultiExecResult }) => void) =>
      sub<{ done: number; total: number; result: MultiExecResult }>('multi-exec-result', cb)
  },
  app: {
    /** Куда приложение реально пишет профиль и логи — видно в настройках. */
    paths: (): Promise<{ config: string; logs: string }> => invoke('app_paths')
  },
  exportText: async (
    content: string,
    defaultName: string
  ): Promise<{ saved: boolean; path?: string }> => {
    const path = await saveDialog({
      title: 'Сохранить отчёт',
      defaultPath: defaultName,
      filters: [{ name: 'Text', extensions: ['txt', 'log'] }]
    })
    if (!path) return { saved: false }
    await invoke('export_text_file', { path, content })
    return { saved: true, path }
  }
}

export type Api = typeof api
