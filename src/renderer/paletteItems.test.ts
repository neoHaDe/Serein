import { describe, expect, it, vi } from 'vitest'
import type { ServerConfig } from '../shared/types'
import { allLeaves, makeLeaf, splitLeaf, type PaneNode } from './paneTree'
import { buildPaletteItems } from './paletteItems'
import { makeServerTab, type Tab } from './tabs'

export function tabWith(key: string, sessions: string[]): Tab {
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

describe('buildPaletteItems', () => {
  it('собирает серверы, действия и вкладки', () => {
    const tabs = [tabWith('a', ['1'])]
    const items = buildPaletteItems([server('s1', 'home')], tabs, 'a', {
      openServer: vi.fn(),
      openLocal: vi.fn(),
      openSettings: vi.fn(),
      openKeyGen: vi.fn(),
      newServer: vi.fn(),
      setWorkspace: vi.fn(),
      focusTab: vi.fn()
    })
    expect(items.some((i) => i.id === 'srv:s1')).toBe(true)
    expect(items.some((i) => i.id === 'act:local')).toBe(true)
    expect(items.some((i) => i.id === 'tab:a')).toBe(true)
  })

  it('добавляет инструменты рабочего места только при SSH на активной вкладке', () => {
    const tab = makeServerTab(server('s1', 'home'))
    const leaf = allLeaves(tab.root)[0]
    leaf.status = 'connected'
    leaf.sessionId = 'sess'
    const items = buildPaletteItems([], [tab], tab.key, {
      openServer: vi.fn(),
      openLocal: vi.fn(),
      openSettings: vi.fn(),
      openKeyGen: vi.fn(),
      newServer: vi.fn(),
      setWorkspace: vi.fn(),
      focusTab: vi.fn()
    })
    expect(items.some((i) => i.id === 'ws:docker')).toBe(true)
  })
})

const server = (id: string, name = id): ServerConfig =>
  ({ id, name, host: 'h', username: 'u', connection: 'ssh' }) as ServerConfig
