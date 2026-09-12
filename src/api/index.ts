/**
 * Мост renderer ↔ Rust: `window.api` через Tauri `invoke` / `listen`.
 * Неперенесённые модули пока возвращают заглушки.
 */
import { Channel, invoke } from '@tauri-apps/api/core'
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
  ServerHardware,
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

export interface BackupProxyCommand {
  serverIndex: number
  serverId?: string
  name: string
  command: string
}

export interface BackupPreview {
  previewed: boolean
  path?: string
  servers?: number
  snippets?: number
  keysRemapped?: number
  proxyCommands?: BackupProxyCommand[]
  contentSha256?: string
}

export interface BackupImportResult {
  imported: boolean
  servers?: number
  snippets?: number
  keysRemapped?: number
  proxyCommands?: BackupProxyCommand[]
  proxyCommandsEnabled?: number
}

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
    importPutty: (): Promise<{ imported: number }> => invoke('servers_import_putty'),
    importMobaxterm: (): Promise<{ imported: number }> => invoke('servers_import_mobaxterm'),
    importXshell: (): Promise<{ imported: number }> => invoke('servers_import_xshell'),
    importSecurecrt: (): Promise<{ imported: number }> => invoke('servers_import_securecrt')
  },
  agent: {
    /** Ключи локального SSH-агента. `ok: false` - агент не запущен, не ошибка вызова. */
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
     * Управляющая команда telnet. `interrupt` - то же, что Ctrl+C на настоящем терминале,
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
    /** Что сессия уже напечатала: новый терминал на живой сессии не должен быть пустым. */
    replay: (id: string): Promise<string> => invoke('session_replay', { id }),
    /** Передать владение сессией окну с указанной меткой. Закрыть её сможет только оно. */
    claim: (id: string, windowLabel: string): Promise<void> =>
      invoke('session_claim', { id, windowLabel }),
    monitor: (id: string): Promise<ServerMetrics> => invoke('session_monitor', { id }),
    /** Железо сервера. Спрашивается один раз: оно не меняется, пока сессия жива. */
    sysinfo: (id: string): Promise<ServerHardware> => invoke('session_sysinfo', { id }),
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
    /** Сервер предъявил незнакомый или изменившийся ключ - ждём решения пользователя. */
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
    preview: async (password: string): Promise<BackupPreview> => {
      const sel = await openDialog({ title: 'Файл бэкапа', filters: [{ name: 'Serein backup', extensions: ['tbk'] }] })
      if (typeof sel !== 'string') return { previewed: false }
      const result = await invoke<Omit<BackupPreview, 'previewed' | 'path'>>('backup_preview', {
        password,
        path: sel
      })
      return { ...result, previewed: true, path: sel }
    },
    import: (
      password: string,
      path: string,
      expectedSha256: string,
      acceptedProxyCommands: number[]
    ): Promise<BackupImportResult> =>
      invoke('backup_import', { password, path, expectedSha256, acceptedProxyCommands })
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
  /**
   * Рабочий стол VNC поверх открытой SSH-сессии.
   *
   * Кадры приходят каналом сырыми байтами, а не ответом команды: экран обновляется
   * десятки раз в секунду, и гонять пиксели через JSON нельзя - разбор формата в
   * `vncFrames.ts`.
   */
  vnc: {
    open: (
      sessionId: string,
      onFrame: (buf: ArrayBuffer) => void,
      opts: { host?: string; port?: number; password?: string } = {}
    ): Promise<string> => {
      const channel = new Channel<ArrayBuffer>()
      channel.onmessage = onFrame
      return invoke('vnc_open', { sessionId, onFrame: channel, ...opts })
    },
    pointer: (id: string, x: number, y: number, buttons: number): Promise<void> =>
      invoke('vnc_pointer', { id, x, y, buttons }),
    key: (id: string, keysym: number, down: boolean): Promise<void> =>
      invoke('vnc_key', { id, keysym, down }),
    refresh: (id: string, full = false): Promise<void> => invoke('vnc_refresh', { id, full }),
    /**
     * Забирает кадры уже открытого сеанса в это окно.
     *
     * Так открепление не рвёт сеанс: он живёт в приложении, а не в окне, и второе окно
     * просто продолжает картинку - без пароля и без переподключения.
     */
    attach: (id: string, onFrame: (buf: ArrayBuffer) => void): Promise<void> => {
      const channel = new Channel<ArrayBuffer>()
      channel.onmessage = onFrame
      return invoke('vnc_attach', { id, onFrame: channel })
    },
    paste: (id: string, text: string): Promise<void> => invoke('vnc_paste', { id, text }),
    close: (id: string): Promise<void> => invoke('vnc_close', { id })
  },

  /**
   * Рабочий стол по RDP.
   *
   * Кадры приходят тем же каналом и в том же формате, что у VNC, поэтому рисует их тот
   * же код. Разбирает протокол отдельный процесс: зависимости IronRDP не сходятся с
   * SSH-ядром в одном дереве, подробности в `src-tauri/src/rdp.rs`.
   */
  rdp: {
    open: (
      sessionId: string,
      onFrame: (buf: ArrayBuffer) => void,
      opts: {
        host?: string
        port?: number
        user: string
        password: string
        domain?: string
        width?: number
        height?: number
        colorDepth?: number
        economy?: boolean
        autologon?: boolean
        networkProfile?: 'vpn' | 'lan'
      }
    ): Promise<string> => {
      const channel = new Channel<ArrayBuffer>()
      channel.onmessage = onFrame
      return invoke('rdp_open', { sessionId, onFrame: channel, ...opts })
    },
    pointer: (id: string, x: number, y: number, buttons: number): Promise<void> =>
      invoke('rdp_pointer', { id, x, y, buttons }),
    /** Код клавиши здесь в терминах RDP, а не X11: раскладку разбирает интерфейс. */
    key: (id: string, code: number, down: boolean): Promise<void> =>
      invoke('rdp_key', { id, code, down }),
    wheel: (id: string, vertical: boolean, delta: number): Promise<void> =>
      invoke('rdp_wheel', { id, vertical, delta }),
    secureAttention: (id: string): Promise<void> => invoke('rdp_secure_attention', { id }),
    /**
     * Просит сервер сменить размер рабочего стола прямо в живом сеансе.
     *
     * Переподключения не будет: размер идёт отдельным каналом управления экраном.
     * Сервер, который его не поддержал, просьбу не заметит - картинка останется
     * вписанной в окно.
     */
    resize: (id: string, width: number, height: number): Promise<void> =>
      invoke('rdp_resize', { id, width, height }),
    /** Забирает кадры уже открытого сеанса в это окно - см. `vnc.attach`. */
    attach: (id: string, onFrame: (buf: ArrayBuffer) => void): Promise<void> => {
      const channel = new Channel<ArrayBuffer>()
      channel.onmessage = onFrame
      return invoke('rdp_attach', { id, onFrame: channel })
    },
    close: (id: string): Promise<void> => invoke('rdp_close', { id }),
    /** Строка от интерфейса в журнал рабочего стола: половину пути кадра из Rust не видно. */
    note: (line: string): Promise<void> => invoke('rdp_note', { line })
  },

  /**
   * Подготовка рабочего стола на сервере: что там есть и чего не хватает.
   *
   * Спрашиваем до подключения, а не после неудачи: «программы нет», «не запущена» и
   * «слушает не там» - три разные беды, и ответ на каждую свой.
   */
  desktop: {
    detect: (
      sessionId: string
    ): Promise<{
      installed: { name: string; path: string }[]
      listening: string[]
      desktop?: string
      packageManager?: string
      sudo?: string
      summary: string
      canInstall: boolean
    }> => invoke('desktop_detect', { sessionId }),
    /**
     * Пароль sudo уходит отдельным полем и попадает на стандартный ввод команды, а не в
     * её строку: строка команды видна в списке процессов сервера всем, кто там есть.
     */
    install: (
      sessionId: string,
      packageManager: string,
      sudoPassword: string
    ): Promise<{ ok: boolean; error?: string }> =>
      invoke('desktop_install', { sessionId, packageManager, sudoPassword }),
    setPassword: (
      sessionId: string,
      password: string
    ): Promise<{ ok: boolean; error?: string }> =>
      invoke('desktop_set_password', { sessionId, password }),

    /**
     * Открыт ли уже рабочий стол у этой сессии. `null` - нет.
     *
     * Спрашивается при открытии панели: сеанс живёт в приложении, а не в окне, и
     * откреплённое окно должно продолжить картинку, а не спрашивать пароль заново.
     */
    active: (
      sessionId: string
    ): Promise<{ kind: 'vnc' | 'rdp'; id: string; width: number; height: number } | null> =>
      invoke('desktop_active', { sessionId }),

    /**
     * То же самое, но про RDP. Отдельным вопросом к серверу, а не одним общим: смотреть
     * надо другое, и общий ответ на двоих путал бы оба.
     */
    rdpDetect: (
      sessionId: string
    ): Promise<{
      installed: { name: string; path: string }[]
      listening: string[]
      service?: string
      desktop?: string
      packageManager?: string
      sudo?: string
      summary: string
      canInstall: boolean
      canStart: boolean
      /** Сервер на Windows: рабочий стол встроен, ставить нечего, пароль sudo не нужен. */
      windows?: boolean
      /** Открыт ли межсетевой экран для рабочего стола. Только для Windows. */
      firewall?: string
    }> => invoke('desktop_rdp_detect', { sessionId }),
    rdpInstall: (
      sessionId: string,
      packageManager: string,
      sudoPassword: string
    ): Promise<{ ok: boolean; error?: string }> =>
      invoke('desktop_rdp_install', { sessionId, packageManager, sudoPassword }),
    /** Включает службу и проверяет, что она действительно поднялась. */
    rdpStart: (
      sessionId: string,
      sudoPassword: string
    ): Promise<{ ok: boolean; error?: string }> =>
      invoke('desktop_rdp_start', { sessionId, sudoPassword })
  },

  /**
   * Базы данных рядом с сервером - через ту же SSH-сессию, а не отдельным соединением.
   * База слушает петлю сервера, и проброс порта для этого поднимать не нужно.
   */
  db: {
    open: (
      sessionId: string,
      params: {
        kind: 'postgres' | 'mysql' | 'redis'
        host?: string
        port?: number
        user?: string
        password?: string
        database?: string
      }
    ): Promise<{ id: string; kind: string; host: string; port: number }> =>
      invoke('db_open', { sessionId, params }),
    query: (
      id: string,
      text: string
    ): Promise<{
      columns: string[]
      rows: Record<string, unknown>[]
      affected: number
      ms: number
      /** Показано не всё: сработал предел на строки, объём ответа или размер ячейки. */
      truncated?: boolean
      /**
       * Все наборы результатов: несколько операторов подряд и хранимые процедуры
       * отвечают не одной выборкой.
       */
      sets?: { columns: string[]; rows: Record<string, unknown>[]; affected: number; truncated?: boolean }[]
      /** Какой набор показывать по умолчанию: первый со строками. */
      shown?: number
    }> => invoke('db_query', { id, text }),
    close: (id: string): Promise<void> => invoke('db_close', { id }),
    /**
     * Уже открытая база этой сессии, если она есть.
     *
     * Спрашиваем приложение, а не свою память: откреплённое окно - отдельный веб-контекст,
     * и памяти о соединении у него нет, а само соединение живёт и переезд окна переживает.
     */
    current: (
      sessionId: string
    ): Promise<{ id: string; kind: string; host: string; port: number } | null> =>
      invoke('db_current', { sessionId })
  },

  workspace: {
    /** Какая система на сервере: от этого зависит набор команд у панелей. */
    platform: (sessionId: string): Promise<{ kind: string; version: string }> =>
      invoke('workspace_platform', { sessionId }),
    processes: (
      sessionId: string
      // `note` - оговорка о том, чего эта система не сообщает: панель показывает её
      // рядом с таблицей, чтобы прочерк в колонке не читался как «ноль».
    ): Promise<{ ok: boolean; error?: string; rows?: WorkspaceProcess[]; note?: string }> =>
      invoke('workspace_processes', { sessionId }),
    kill: (sessionId: string, pid: number): Promise<{ ok: boolean; error?: string }> =>
      invoke('workspace_kill', { sessionId, pid }),
    services: (
      sessionId: string
    ): Promise<{ ok: boolean; error?: string; rows?: WorkspaceService[]; note?: string }> =>
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
     * но результат каждого хоста приходит событием сразу - ждать самый медленный,
     * чтобы увидеть первый, незачем.
     */
    exec: (serverIds: string[], command: string): Promise<MultiExecResult[]> =>
      invoke('multi_exec', { serverIds, command }),
    cancel: (): Promise<void> => invoke('multi_exec_cancel'),
    onResult: (cb: (p: { done: number; total: number; result: MultiExecResult }) => void) =>
      sub<{ done: number; total: number; result: MultiExecResult }>('multi-exec-result', cb)
  },
  tools: {
    portTest: (host: string, port: number, timeoutMs?: number): Promise<Record<string, unknown>> =>
      invoke('tools_port_test', { host, port, timeoutMs }),
    dnsLookup: (name: string): Promise<Record<string, unknown>> => invoke('tools_dns_lookup', { name }),
    /**
     * То же самое, но глазами сервера.
     *
     * При разборе неполадки почти всегда важно не «вижу ли я адрес», а «видит ли его
     * сервер»: у него свои маршруты, свой DNS и свой `/etc/hosts`.
     */
    portTestOn: (sessionId: string, host: string, port: number): Promise<Record<string, unknown>> =>
      invoke('tools_port_test_on', { sessionId, host, port }),
    dnsLookupOn: (sessionId: string, name: string): Promise<Record<string, unknown>> =>
      invoke('tools_dns_lookup_on', { sessionId, name }),
    /**
     * Запрос к каталогу LDAP. Только с этой машины: готовый клиент открытый поток не
     * принимает, а писать разбор ASN.1 ради варианта «с сервера» несоразмерно пользе.
     */
    ldap: (params: {
      url: string
      bindDn?: string
      password?: string
      base?: string
      filter?: string
    }): Promise<Record<string, unknown>> => invoke('tools_ldap', { params }),
    /**
     * Сравнение двух файлов. Каждая сторона - эта машина (`sessionId` пуст) либо открытая
     * сессия. Смысл именно в разнородности: «тот же ли конфиг на двух серверах».
     */
    diff: (
      a: { sessionId?: string; path: string },
      b: { sessionId?: string; path: string }
    ): Promise<Record<string, unknown>> => invoke('tools_diff', { a, b }),
    /**
     * Выбрать файл на этой машине системным диалогом.
     *
     * `null` - человек передумал. Отличать это от ошибки важно: молча ничего не делать
     * при отказе правильно, а при ошибке - нет.
     */
    pickLocalFile: async (): Promise<string | null> => {
      const sel = await openDialog({ multiple: false, directory: false, title: 'Выберите файл' })
      return typeof sel === 'string' ? sel : null
    },
    /** Просмотр диапазона портов. За раз - не больше 1024, это ограничение по смыслу. */
    portScan: (host: string, from: number, to: number): Promise<Record<string, unknown>> =>
      invoke('tools_port_scan', { host, from, to }),
    /**
     * HTTP-запрос: код ответа, заголовки, время, цепочка переходов.
     *
     * Со своей машины запрос делается своими силами и показывает каждый переход отдельно.
     * С сервера - через `curl` или `wget`, и там виден только итог: чужими программами
     * цепочку не разложить.
     */
    http: (url: string, method?: string, maxRedirects?: number): Promise<Record<string, unknown>> =>
      invoke('tools_http', { url, method, maxRedirects }),
    httpOn: (sessionId: string, url: string, method?: string): Promise<Record<string, unknown>> =>
      invoke('tools_http_on', { sessionId, url, method }),
    /** Маршрут до адреса. `hops` - предел числа узлов, по умолчанию 15. */
    trace: (host: string, hops?: number): Promise<Record<string, unknown>> =>
      invoke('tools_trace', { host, hops }),
    traceOn: (sessionId: string, host: string, hops?: number): Promise<Record<string, unknown>> =>
      invoke('tools_trace_on', { sessionId, host, hops }),
    portScanOn: (
      sessionId: string,
      host: string,
      from: number,
      to: number
    ): Promise<Record<string, unknown>> =>
      invoke('tools_port_scan_on', { sessionId, host, from, to }),
    tlsCert: (host: string, port?: number): Promise<Record<string, unknown>> =>
      invoke('tools_tls_cert', { host, port }),
    subnet: (input: string): Promise<Record<string, unknown>> => invoke('tools_subnet', { input }),
    hash: (algo: string, text: string): Promise<Record<string, unknown>> => invoke('tools_hash', { algo, text }),
    jwtDecode: (token: string): Promise<Record<string, unknown>> => invoke('tools_jwt_decode', { token })
  },
  app: {
    /** Куда приложение реально пишет профиль и логи - видно в настройках. */
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
