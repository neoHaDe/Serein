import { describe, expect, it } from 'vitest'
import type { SavedAuxWindow, SerializedTab, ServerConfig } from '../shared/types'
import { planReattach, planRestore } from './tabs'

/**
 * Первые тесты на фронтенде вообще. Взяты именно за восстановление запуска: логика тут
 * ветвится по двум независимым настройкам, а проверить её раньше было нельзя — она сидела
 * внутри `useEffect` посреди `App.tsx`.
 */

const leaf = (serverId?: string): SerializedTab['root'] =>
  ({ t: 'leaf', kind: serverId ? 'ssh' : 'local', title: 'вкладка', serverId }) as SerializedTab['root']

const savedTab = (over: Partial<SerializedTab> = {}): SerializedTab =>
  ({ title: 'вкладка', root: leaf('srv-1'), sftpOpen: false, workspace: 'terminal', ...over }) as SerializedTab

const server = (id: string, name = id): ServerConfig =>
  ({ id, name, host: 'h', username: 'u', connection: 'ssh' }) as ServerConfig

const auxSftp = (serverId: string): SavedAuxWindow =>
  ({ kind: 'sftp', serverId, x: 0, y: 0, w: 480, h: 720 }) as SavedAuxWindow

describe('planRestore', () => {
  it('без обеих настроек не открывает ничего', () => {
    const tabs = planRestore({
      restoreTabs: false,
      restoreAux: false,
      saved: [savedTab()],
      auxWindows: [auxSftp('srv-1')],
      servers: [server('srv-1')]
    })
    expect(tabs).toHaveLength(0)
  })

  it('с настройкой вкладок открывает всё сохранённое', () => {
    const tabs = planRestore({
      restoreTabs: true,
      restoreAux: false,
      saved: [savedTab({ title: 'первая' }), savedTab({ title: 'вторая' })],
      auxWindows: [],
      servers: []
    })
    expect(tabs.map((t) => t.title)).toEqual(['первая', 'вторая'])
  })

  it('без вкладок, но с доп. окнами поднимает вкладку под окно', () => {
    // Окно SFTP живёт поверх SSH-сессии, а сессия берётся из вкладки. Значит вкладка
    // нужна, даже если пользователь не просил восстанавливать вкладки.
    const tabs = planRestore({
      restoreTabs: false,
      restoreAux: true,
      saved: [savedTab({ title: 'нужная' })],
      auxWindows: [auxSftp('srv-1')],
      servers: [server('srv-1')]
    })
    expect(tabs).toHaveLength(1)
    expect(tabs[0].title).toBe('нужная')
    // Рабочее пространство при этом НЕ восстанавливаем: вкладка нужна только как носитель
    // сессии, разворачивать в ней файловый менеджер пользователь не просил.
    expect(tabs[0].sftpOpen).toBe(false)
  })

  it('собирает вкладку из профиля, если сохранённой не нашлось', () => {
    const tabs = planRestore({
      restoreTabs: false,
      restoreAux: true,
      saved: [],
      auxWindows: [auxSftp('srv-2')],
      servers: [server('srv-2', 'из профиля')]
    })
    expect(tabs).toHaveLength(1)
    expect(tabs[0].title).toBe('из профиля')
  })

  it('не открывает две вкладки на один сервер', () => {
    // Два окна на одном сервере — это одна сессия, а не две.
    const tabs = planRestore({
      restoreTabs: false,
      restoreAux: true,
      saved: [savedTab()],
      auxWindows: [auxSftp('srv-1'), auxSftp('srv-1')],
      servers: [server('srv-1')]
    })
    expect(tabs).toHaveLength(1)
  })

  it('пропускает окно, для которого сервера уже нет', () => {
    // Профиль удалили, а окно в раскладке осталось — подключаться некуда.
    const tabs = planRestore({
      restoreTabs: false,
      restoreAux: true,
      saved: [],
      auxWindows: [auxSftp('удалённый')],
      servers: []
    })
    expect(tabs).toHaveLength(0)
  })
})

describe('planReattach', () => {
  const req = {
    sessionId: 'сессия-1',
    serverId: 'srv-1',
    title: 'home-srv',
    workspace: 'terminal' as const,
    sftpOpen: false,
    kind: 'ssh' as const
  }

  it('создаёт вкладку, если её нет', () => {
    const { tabs, activeKey } = planReattach([], req)
    expect(tabs).toHaveLength(1)
    expect(tabs[0].key).toBe(activeKey)
    // Сессия уже живёт — вкладка обязана появиться сразу подключённой, иначе интерфейс
    // покажет «отключено» и полезет открывать второе соединение.
    expect(tabs[0].root).toMatchObject({ sessionId: 'сессия-1', status: 'connected' })
  })

  it('не заводит вторую вкладку на ту же сессию', () => {
    // Вернули не всю вкладку, а только панель: вкладка с этой сессией никуда не девалась.
    // Две вкладки на одну сессию — это когда закрытие любой уносит её у соседа.
    const first = planReattach([], req).tabs
    const { tabs, activeKey } = planReattach(first, { ...req, workspace: 'docker' })
    expect(tabs).toHaveLength(1)
    expect(activeKey).toBe(first[0].key)
    expect(tabs[0].workspace).toBe('docker')
  })

  it('переносит на существующую вкладку заголовок и файловый менеджер', () => {
    const first = planReattach([], req).tabs
    const { tabs } = planReattach(first, { ...req, title: 'новое имя', sftpOpen: true })
    expect(tabs[0].title).toBe('новое имя')
    expect(tabs[0].sftpOpen).toBe(true)
  })

  it('не трогает чужие вкладки', () => {
    const other = planReattach([], { ...req, sessionId: 'другая', title: 'другая' }).tabs
    const { tabs } = planReattach(other, req)
    expect(tabs).toHaveLength(2)
    expect(tabs[0].title).toBe('другая')
  })
})
