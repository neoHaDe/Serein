import { describe, it, expect } from 'vitest'
import type { SerializedPane, SerializedTab, ServerConfig, WorkspaceProfile } from '../shared/types'
import { nameProblem, profileInfo, profileSummary, withoutMissing } from './workspaceProfiles'

const srv = (id: string, name: string): ServerConfig => ({ id, name } as ServerConfig)
const leaf = (serverId?: string): SerializedPane => ({ t: 'leaf', kind: serverId ? 'ssh' : 'local', serverId, title: serverId ?? 'local' })
const split = (a: SerializedPane, b: SerializedPane): SerializedPane => ({ t: 'split', dir: 'row', sizes: [50, 50], children: [a, b] })
const tab = (root: SerializedPane, title = 't'): SerializedTab => ({ title, root, workspace: 'terminal' })
const profile = (tabs: SerializedTab[], name = 'Продакшн', id = 'p1'): WorkspaceProfile => ({ id, name, tabs, savedAt: 1 })

const servers = [srv('db', 'prod-db'), srv('web', 'web-1')]

describe('профили рабочего пространства', () => {
  it('сводка называет серверы, локальные терминалы и потерянные серверы', () => {
    const p = profile([tab(split(leaf('db'), leaf('web'))), tab(leaf('db')), tab(split(leaf(), leaf('gone')))])
    const info = profileInfo(p, servers)
    expect(info).toEqual({ tabs: 3, servers: ['prod-db', 'web-1'], local: 1, missing: 1 })
    expect(profileSummary(info)).toBe('3 вкладки · prod-db, web-1 · локальных терминалов: 1 · удалённых серверов: 1')
  })

  it('панели на удалённые серверы отбрасываются, пустые вкладки - тоже', () => {
    const tabs = withoutMissing([tab(split(leaf('db'), leaf('gone')), 'a'), tab(leaf('gone'), 'b'), tab(leaf(), 'c')], servers)
    expect(tabs.map((t) => t.title)).toEqual(['a', 'c'])
    expect(tabs[0].root).toEqual(leaf('db'))
  })

  it('название обязательно и не повторяется без учёта регистра', () => {
    const list = [profile([], 'Продакшн', 'p1')]
    expect(nameProblem('  ', list)).toBe('Нужно название')
    expect(nameProblem('продакшн', list)).toContain('уже есть')
    expect(nameProblem('Продакшн', list, 'p1')).toBeNull()
    expect(nameProblem('Логи', list)).toBeNull()
  })
})
