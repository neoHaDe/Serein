import { describe, expect, it } from 'vitest'
import { makeLeaf, splitLeaf } from './paneTree'
import type { PaneLeaf, PaneNode } from './paneTree'
import type { Tab } from './tabs'
import { aggregateServerStatuses } from './serverStatus'

type LeafSpec = { serverId?: string; status?: PaneLeaf['status']; kind?: PaneLeaf['kind'] }

function tab(key: string, specs: LeafSpec[]): Tab {
  const build = (s: LeafSpec): PaneLeaf => {
    const l = makeLeaf(s.kind ?? 'ssh', key, s.serverId)
    l.status = s.status ?? 'connected'
    return l
  }
  const first = build(specs[0])
  let root: PaneNode = first
  for (const s of specs.slice(1)) root = splitLeaf(root, first.id, 'row', build(s))
  return { key, title: key, kind: 'terminal', root, activePaneId: first.id, sftpOpen: false, workspace: 'terminal' }
}

describe('aggregateServerStatuses', () => {
  it('берёт лучший статус, а не последний встреченный', () => {
    // Сервер открыт дважды: в одной вкладке работает, в другой сорвалось. Точка отвечает
    // на вопрос «есть ли рабочее соединение», и ответ здесь — да.
    const tabs = [
      tab('первая', [{ serverId: 'srv', status: 'error' }]),
      tab('вторая', [{ serverId: 'srv', status: 'connected' }])
    ]
    expect(aggregateServerStatuses(tabs)).toEqual({ srv: 'connected' })
    // Порядок вкладок не должен влиять на ответ.
    expect(aggregateServerStatuses([...tabs].reverse())).toEqual({ srv: 'connected' })
  })

  it('различает промежуточные состояния по приоритету', () => {
    const tabs = [
      tab('a', [{ serverId: 'srv', status: 'connecting' }, { serverId: 'srv', status: 'reconnecting' }])
    ]
    expect(aggregateServerStatuses(tabs)).toEqual({ srv: 'reconnecting' })
  })

  it('закрытую панель не показывает вовсе', () => {
    // Закрытая панель — это не состояние сервера, а её отсутствие.
    const tabs = [tab('a', [{ serverId: 'srv', status: 'closed' }])]
    expect(aggregateServerStatuses(tabs)).toEqual({})
  })

  it('не путает серверы между собой', () => {
    const tabs = [
      tab('a', [{ serverId: 'один', status: 'connected' }, { serverId: 'два', status: 'error' }])
    ]
    expect(aggregateServerStatuses(tabs)).toEqual({ один: 'connected', два: 'error' })
  })

  it('игнорирует не-SSH и панели без сервера', () => {
    const tabs = [
      tab('a', [{ kind: 'local', status: 'connected' }, { serverId: 'srv', kind: 'serial', status: 'connected' }])
    ]
    expect(aggregateServerStatuses(tabs)).toEqual({})
  })
})
