import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { AppSettings, ServerConfig } from '../../shared/types'
import { applyDropServerOrder, mergeGroupNames, planDropServer, planMoveGroup } from '../serverGroups'
import { errText } from '../errText'

/**
 * Серверы в сайдбаре: список, CRUD, импорт, группы и порядок.
 *
 * Жило в `App.tsx` вперемешку с вкладками и терминалами. Отдельно — потому что это
 * другой домен: профили подключений и их раскладка в списке, без привязки к сессиям.
 */

export type ServerImportKind = 'ssh' | 'putty' | 'mobaxterm' | 'xshell' | 'securecrt'

export interface Operations {
  servers: ServerConfig[]
  setServers: React.Dispatch<React.SetStateAction<ServerConfig[]>>
  reloadServers: () => Promise<void>
  saveServer: (cfg: ServerConfig) => Promise<void>
  /** Точечная правка профиля: избранное, среда, теги — без открытия формы. */
  patchServer: (id: string, patch: Partial<ServerConfig>) => Promise<void>
  deleteServer: (id: string) => Promise<void>
  importServers: (kind: ServerImportKind) => Promise<void>
  groupOrder: string[]
  collapsedGroups: string[]
  allGroups: string[]
  saveGroupOrder: (next: string[]) => void
  toggleGroup: (group: string) => void
  createGroup: () => void
  dropServer: (serverId: string, group: string, index?: number) => Promise<void>
  dropGroup: (order: string[]) => void
  renameGroup: (from: string, to: string) => Promise<void>
  deleteGroup: (group: string) => Promise<void>
  moveGroup: (group: string, dir: -1 | 1) => void
}

export function useOperations(
  settings: AppSettings,
  update: (patch: Partial<AppSettings>) => void,
  onSavedServer: () => void
): Operations {
  const [servers, setServers] = useState<ServerConfig[]>([])
  const settingsRef = useRef(settings)
  settingsRef.current = settings

  const reloadServers = useCallback(async () => {
    setServers(await window.api.servers.list())
  }, [])

  useEffect(() => {
    void reloadServers()
  }, [reloadServers])

  const saveServer = useCallback(
    async (cfg: ServerConfig) => {
      await window.api.servers.save(cfg)
      await reloadServers()
      onSavedServer()
    },
    [reloadServers, onSavedServer]
  )

  const serversRef = useRef(servers)
  serversRef.current = servers

  /**
   * Меняет одно-два поля профиля и сохраняет его.
   *
   * Секреты при этом не теряются: хранилище перезаписывает пароль только тогда, когда ключ
   * пришёл явно, а в списке серверов его нет. Поэтому патчить безопасно.
   */
  const patchServer = useCallback(
    async (id: string, patch: Partial<ServerConfig>) => {
      const cur = serversRef.current.find((s) => s.id === id)
      if (!cur) return
      await window.api.servers.save({ ...cur, ...patch })
      await reloadServers()
    },
    [reloadServers]
  )

  const deleteServer = useCallback(
    async (id: string) => {
      await window.api.servers.remove(id)
      await reloadServers()
    },
    [reloadServers]
  )

  const importServers = useCallback(
    async (kind: ServerImportKind) => {
      try {
        const handlers: Record<ServerImportKind, () => Promise<{ imported: number }>> = {
          ssh: () => window.api.servers.importSshConfig(),
          putty: () => window.api.servers.importPutty(),
          mobaxterm: () => window.api.servers.importMobaxterm(),
          xshell: () => window.api.servers.importXshell(),
          securecrt: () => window.api.servers.importSecurecrt()
        }
        const r = await handlers[kind]()
        await reloadServers()
        alert(
          r.imported
            ? `Импортировано серверов: ${r.imported}`
            : 'Новых серверов не нашлось — всё уже есть в списке.'
        )
      } catch (e) {
        alert(errText(e))
      }
    },
    [reloadServers]
  )

  const groupOrder = useMemo(() => settings.groupOrder ?? [], [settings.groupOrder])
  const collapsedGroups = useMemo(() => settings.collapsedGroups ?? [], [settings.collapsedGroups])
  const allGroups = useMemo(() => mergeGroupNames(groupOrder, servers), [groupOrder, servers])

  const saveGroupOrder = useCallback(
    (next: string[]) => {
      update({ groupOrder: next })
    },
    [update]
  )

  const toggleGroup = useCallback(
    (group: string) => {
      const cur = settingsRef.current.collapsedGroups ?? []
      update({
        collapsedGroups: cur.includes(group) ? cur.filter((g) => g !== group) : [...cur, group]
      })
    },
    [update]
  )

  const createGroup = useCallback(() => {
    const name = prompt('Название новой группы')?.trim()
    if (!name) return
    const cur = settingsRef.current.groupOrder ?? []
    if (cur.includes(name)) {
      alert(`Группа «${name}» уже есть`)
      return
    }
    update({ groupOrder: [...cur, name] })
  }, [update])

  const dropServer = useCallback(
    async (serverId: string, group: string, index?: number) => {
      const order = planDropServer(servers, serverId, group, index)
      if (!order) return
      setServers((prev) => applyDropServerOrder(prev, order))
      await window.api.servers.reorder(order)
      await reloadServers()
    },
    [servers, reloadServers]
  )

  const dropGroup = useCallback(
    (order: string[]) => {
      update({ groupOrder: order })
    },
    [update]
  )

  const renameGroup = useCallback(
    async (from: string, to: string) => {
      const members = servers.filter((s) => (s.group?.trim() || '') === from)
      if (members.length) {
        await window.api.servers.reorder(
          members.map((s, i) => ({ id: s.id, group: to, order: s.order ?? i }))
        )
      }
      const cur = settingsRef.current.groupOrder ?? []
      update({ groupOrder: cur.map((g) => (g === from ? to : g)) })
      await reloadServers()
    },
    [servers, update, reloadServers]
  )

  const deleteGroup = useCallback(
    async (group: string) => {
      const members = servers.filter((s) => (s.group?.trim() || '') === group)
      if (members.length) {
        await window.api.servers.reorder(members.map((s, i) => ({ id: s.id, group: '', order: i })))
      }
      const cur = settingsRef.current.groupOrder ?? []
      update({ groupOrder: cur.filter((g) => g !== group) })
      await reloadServers()
    },
    [servers, update, reloadServers]
  )

  const moveGroup = useCallback(
    (group: string, dir: -1 | 1) => {
      const next = planMoveGroup(settingsRef.current.groupOrder ?? [], allGroups, group, dir)
      if (next) update({ groupOrder: next })
    },
    [update, allGroups]
  )

  return {
    servers,
    setServers,
    reloadServers,
    saveServer,
    patchServer,
    deleteServer,
    importServers,
    groupOrder,
    collapsedGroups,
    allGroups,
    saveGroupOrder,
    toggleGroup,
    createGroup,
    dropServer,
    dropGroup,
    renameGroup,
    deleteGroup,
    moveGroup
  }
}
