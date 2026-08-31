import { describe, expect, it } from 'vitest'
import type { SavedAuxWindow, SerializedTab, ServerConfig } from '../shared/types'
import type { PaneNode } from './paneTree'
import { allLeaves, makeLeaf, splitLeaf } from './paneTree'
import type { Tab } from './tabs'
import {
  broadcastTargets,
  canDetachTab,
  closePaneIn,
  makeLocalTab,
  makeServerTab,
  nextActivePaneId,
  nextTabKey,
  openSftpOnTab,
  planReattach,
  planReattachSftp,
  planReattachWorkspace,
  planRestore,
  planSplitPane,
  reorderTabs,
  setTabWorkspace,
  sshLeafForTools,
  toggleSftpOnTab
} from './tabs'
import { applyDropServerOrder, mergeGroupNames, planDropServer, planMoveGroup } from './serverGroups'

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

/** Вкладка с панелями на заданных сессиях (первая — активная). */
function tabWith(key: string, sessions: string[]): Tab {
  const first = makeLeaf('ssh', key, 'srv')
  first.sessionId = sessions[0]
  let root: PaneNode = first
  for (const s of sessions.slice(1)) {
    const leaf = makeLeaf('ssh', key, 'srv')
    leaf.sessionId = s
    root = splitLeaf(root, first.id, 'row', leaf)
  }
  return {
    key,
    title: key,
    kind: 'terminal',
    root,
    activePaneId: first.id,
    sftpOpen: false,
    workspace: 'terminal'
  }
}

describe('broadcastTargets', () => {
  it('не выходит за пределы своей вкладки', () => {
    // Главное правило: вкладки держат разные окружения, и рассылка по всем отправила бы
    // команду в прод из вкладки, открытой на тесте. Отменить такое нельзя.
    const tabs = [tabWith('тест', ['тест-1', 'тест-2']), tabWith('прод', ['прод-1', 'прод-2'])]
    // Из тестовой вкладки — только по тестовым панелям, прод не задет ни при каких условиях.
    expect(broadcastTargets(tabs, 'тест-1')).toEqual(['тест-2'])
    expect(broadcastTargets(tabs, 'прод-1')).toEqual(['прод-2'])
  })

  it('доходит до соседних панелей своей вкладки', () => {
    const tabs = [tabWith('одна', ['а', 'б', 'в'])]
    expect(broadcastTargets(tabs, 'а').sort()).toEqual(['б', 'в'])
  })

  it('не отправляет ввод обратно источнику', () => {
    const tabs = [tabWith('одна', ['своя'])]
    expect(broadcastTargets(tabs, 'своя')).toEqual([])
  })

  it('для неизвестной сессии не возвращает ничего', () => {
    const tabs = [tabWith('одна', ['своя'])]
    expect(broadcastTargets(tabs, 'чужая')).toEqual([])
  })
})

describe('closePaneIn', () => {
  it('последняя панель закрывает вкладку целиком', () => {
    // Вкладка без единой панели выглядела бы сломанной.
    const tabs = [tabWith('одна', ['s1'])]
    const res = closePaneIn(tabs, 'одна', tabs[0].activePaneId)
    expect(res.tabs).toHaveLength(0)
    expect(res.closedTab).toBe(true)
  })

  it('закрытие активной панели переводит активность на оставшуюся', () => {
    // Иначе вкладка ссылается на панель, которой уже нет.
    const tabs = [tabWith('одна', ['s1', 's2'])]
    const active = tabs[0].activePaneId
    const res = closePaneIn(tabs, 'одна', active)
    expect(res.closedTab).toBe(false)
    expect(res.tabs).toHaveLength(1)
    expect(res.tabs[0].activePaneId).not.toBe(active)
    expect(allLeaves(res.tabs[0].root).map((l) => l.sessionId)).toEqual(['s2'])
  })

  it('не трогает соседние вкладки', () => {
    const tabs = [tabWith('первая', ['s1']), tabWith('вторая', ['s2'])]
    const res = closePaneIn(tabs, 'первая', tabs[0].activePaneId)
    expect(res.tabs.map((t) => t.key)).toEqual(['вторая'])
  })

  it('на неизвестной панели ничего не меняет', () => {
    const tabs = [tabWith('одна', ['s1'])]
    const res = closePaneIn(tabs, 'одна', 'нет-такой-панели')
    expect(res.tabs).toHaveLength(1)
    expect(res.closedTab).toBe(false)
  })
})

describe('sshLeafForTools', () => {
  it('следует за активной панелью, если она SSH', () => {
    // Инструменты относятся к тому, на что человек смотрит.
    const t = tabWith('одна', ['первая', 'вторая'])
    const leaf = sshLeafForTools(t)
    expect(leaf?.sessionId).toBe('первая')
  })

  it('когда активен локальный терминал, берёт подключённую SSH-панель', () => {
    const local = makeLeaf('local', 'локальный')
    const ssh = makeLeaf('ssh', 'сервер', 'srv')
    ssh.sessionId = 'сессия'
    ssh.status = 'connected'
    const root = splitLeaf(local, local.id, 'row', ssh)
    const t: Tab = {
      key: 'k', title: 'k', kind: 'terminal', root,
      activePaneId: local.id, sftpOpen: false, workspace: 'terminal'
    }
    expect(sshLeafForTools(t)?.sessionId).toBe('сессия')
  })

  it('берёт неподключённую SSH-панель, если других нет', () => {
    // Рельс должен показать статус и кнопку переподключения, а не исчезнуть молча.
    const local = makeLeaf('local', 'локальный')
    const ssh = makeLeaf('ssh', 'сервер', 'srv')
    ssh.status = 'error'
    const root = splitLeaf(local, local.id, 'row', ssh)
    const t: Tab = {
      key: 'k', title: 'k', kind: 'terminal', root,
      activePaneId: local.id, sftpOpen: false, workspace: 'terminal'
    }
    expect(sshLeafForTools(t)?.id).toBe(ssh.id)
  })

  it('без SSH-панелей не возвращает ничего', () => {
    const local = makeLeaf('local', 'локальный')
    const t: Tab = {
      key: 'k', title: 'k', kind: 'terminal', root: local,
      activePaneId: local.id, sftpOpen: false, workspace: 'terminal'
    }
    expect(sshLeafForTools(t)).toBeUndefined()
  })
})

describe('reorderTabs', () => {
  it('переставляет вкладку на место другой', () => {
    const tabs = [tabWith('a', ['1']), tabWith('b', ['2']), tabWith('c', ['3'])]
    expect(reorderTabs(tabs, 'c', 'a').map((t) => t.key)).toEqual(['c', 'a', 'b'])
  })

  it('не трогает список при переносе на себя или неизвестном ключе', () => {
    const tabs = [tabWith('a', ['1']), tabWith('b', ['2'])]
    expect(reorderTabs(tabs, 'a', 'a')).toBe(tabs)
    expect(reorderTabs(tabs, 'нет', 'a')).toBe(tabs)
    expect(reorderTabs(tabs, 'a', 'нет')).toBe(tabs)
  })
})

describe('makeServerTab / makeLocalTab', () => {
  it('создаёт терминальную вкладку под сервер', () => {
    const tab = makeServerTab(server('srv-1', 'home'))
    expect(tab.kind).toBe('terminal')
    expect(tab.title).toBe('home')
    expect(tab.workspace).toBe('terminal')
    expect(allLeaves(tab.root)[0].serverId).toBe('srv-1')
  })

  it('создаёт локальную вкладку без serverId', () => {
    const tab = makeLocalTab()
    expect(tab.title).toBe('Локальный терминал')
    expect(allLeaves(tab.root)[0].kind).toBe('local')
  })
})

describe('planSplitPane', () => {
  it('добавляет локальную панель при split', () => {
    const tab = makeLocalTab()
    const next = planSplitPane(tab, [], 'row', { kind: 'local' })
    expect(allLeaves(next.root)).toHaveLength(2)
    expect(next.activePaneId).not.toBe(tab.activePaneId)
  })

  it('берёт вид панели из профиля сервера', () => {
    const tab = makeServerTab({ ...server('s1'), connection: 'telnet' } as ServerConfig)
    const next = planSplitPane(tab, [{ ...server('s1'), connection: 'telnet' } as ServerConfig], 'col', {
      kind: 'ssh',
      serverId: 's1',
      title: 'tel'
    })
    const added = allLeaves(next.root).find((l) => l.id === next.activePaneId)
    expect(added?.kind).toBe('telnet')
  })
})

describe('canDetachTab', () => {
  it('разрешает одну подключённую SSH-панель', () => {
    const tab = tabWith('одна', ['s1'])
    const leaf = allLeaves(tab.root)[0]
    leaf.status = 'connected'
    expect(canDetachTab(tab).ok).toBe(true)
  })

  it('блокирует несколько панелей', () => {
    const tab = tabWith('две', ['s1', 's2'])
    expect(canDetachTab(tab)).toEqual({ ok: false, reason: 'multi-pane' })
  })

  it('блокирует неподключённую панель', () => {
    const tab = makeServerTab(server('s1'))
    expect(canDetachTab(tab)).toEqual({ ok: false, reason: 'not-connected' })
  })
})

describe('toggleSftpOnTab / openSftpOnTab / setTabWorkspace', () => {
  it('toggle открывает и закрывает SFTP', () => {
    const tab = tabWith('a', ['1'])
    const open = toggleSftpOnTab([tab], 'a')
    expect(open[0].sftpOpen).toBe(true)
    expect(toggleSftpOnTab(open, 'a')[0].sftpOpen).toBe(false)
  })

  it('open всегда открывает SFTP', () => {
    const tab = tabWith('a', ['1'])
    expect(openSftpOnTab([tab], 'a')[0].sftpOpen).toBe(true)
    expect(openSftpOnTab(openSftpOnTab([tab], 'a'), 'a')[0].sftpOpen).toBe(true)
  })

  it('setTabWorkspace меняет инструмент только у нужной вкладки', () => {
    const tabs = [tabWith('a', ['1']), tabWith('b', ['2'])]
    const next = setTabWorkspace(tabs, 'b', 'docker')
    expect(next[0].workspace).toBe('terminal')
    expect(next[1].workspace).toBe('docker')
  })
})

describe('nextTabKey / nextActivePaneId', () => {
  it('переключает вкладки по кругу', () => {
    const tabs = [tabWith('a', ['1']), tabWith('b', ['2'])]
    expect(nextTabKey(tabs, 'a', 1)).toBe('b')
    expect(nextTabKey(tabs, 'b', 1)).toBe('a')
  })

  it('переключает панели внутри вкладки', () => {
    const tab = tabWith('a', ['1', '2'])
    const first = tab.activePaneId
    const second = nextActivePaneId(tab, 1)!
    expect(second).not.toBe(first)
    expect(nextActivePaneId({ ...tab, activePaneId: second }, 1)).toBe(first)
  })
})

describe('planReattachWorkspace / planReattachSftp', () => {
  it('workspace: фокус на существующей вкладке', () => {
    const tabs = planReattach([], {
      sessionId: 's1',
      serverId: 'srv',
      title: 'h',
      workspace: 'terminal',
      sftpOpen: false,
      kind: 'ssh'
    }).tabs
    expect(planReattachWorkspace(tabs, { sessionId: 's1', title: 'h', tool: 'docker' })).toEqual({
      action: 'focus',
      key: tabs[0].key,
      workspace: 'docker'
    })
  })

  it('sftp: вставка новой вкладки, если сессии ещё нет', () => {
    expect(
      planReattachSftp([], { sessionId: 's1', serverId: 'srv' }, 'home').action
    ).toBe('insert')
  })
})

describe('serverGroups', () => {
  it('mergeGroupNames объединяет настройки и серверы без дублей', () => {
    expect(
      mergeGroupNames(['prod'], [
        server('a'),
        { ...server('b'), group: 'stage' } as ServerConfig,
        { ...server('c'), group: 'prod' } as ServerConfig
      ])
    ).toEqual(['prod', 'stage'])
  })

  it('planDropServer вставляет сервер в группу', () => {
    const servers = [
      { ...server('a'), group: 'g', order: 0 } as ServerConfig,
      { ...server('b'), group: 'g', order: 1 } as ServerConfig,
      { ...server('c'), group: 'other', order: 0 } as ServerConfig
    ]
    expect(planDropServer(servers, 'c', 'g', 0)).toEqual([
      { id: 'c', group: 'g', order: 0 },
      { id: 'a', group: 'g', order: 1 },
      { id: 'b', group: 'g', order: 2 }
    ])
  })

  it('applyDropServerOrder обновляет только затронутые серверы', () => {
    const servers = [
      { ...server('a'), group: 'g', order: 0 } as ServerConfig,
      { ...server('b'), group: 'other', order: 0 } as ServerConfig
    ]
    const next = applyDropServerOrder(servers, [{ id: 'a', group: 'g', order: 2 }])
    expect(next[0].order).toBe(2)
    expect(next[1].order).toBe(0)
  })

  it('planMoveGroup меняет порядок групп', () => {
    expect(planMoveGroup(['a', 'b', 'c'], [], 'b', -1)).toEqual(['b', 'a', 'c'])
    expect(planMoveGroup(['a', 'b'], [], 'a', -1)).toBeNull()
  })
})
