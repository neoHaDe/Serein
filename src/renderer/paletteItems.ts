import type { ServerConfig, WorkspaceTool } from '../shared/types'
import type { PaletteItem } from './components/CommandPalette'
import type { Tab } from './tabs'
import { sshLeafForTools } from './tabs'

const WORKSPACE_TOOLS: { id: WorkspaceTool; label: string }[] = [
  { id: 'terminal', label: 'Terminal' },
  { id: 'overview', label: 'Overview' },
  { id: 'docker', label: 'Docker' },
  { id: 'logs', label: 'Logs' },
  { id: 'processes', label: 'Processes' },
  { id: 'services', label: 'Services' },
  { id: 'tunnels', label: 'Tunnels' }
]

export interface PaletteActions {
  openServer: (server: ServerConfig) => void
  openLocal: () => void
  openSettings: () => void
  openKeyGen: () => void
  newServer: () => void
  setWorkspace: (tabKey: string, tool: WorkspaceTool) => void
  focusTab: (tabKey: string) => void
}

/** Пункты командной палитры — чистая сборка, run-колбэки приходят снаружи. */
export function buildPaletteItems(
  servers: ServerConfig[],
  tabs: Tab[],
  activeKey: string | null,
  actions: PaletteActions
): PaletteItem[] {
  const items: PaletteItem[] = []
  for (const s of servers) {
    items.push({
      id: 'srv:' + s.id,
      label: s.name,
      hint: `${s.username}@${s.host}`,
      icon: '🔌',
      group: 'Сервер',
      run: () => actions.openServer(s)
    })
  }
  items.push({
    id: 'act:local',
    label: 'Новый локальный терминал',
    icon: '🖥',
    group: 'Действие',
    run: actions.openLocal
  })
  items.push({
    id: 'act:settings',
    label: 'Настройки',
    icon: '⚙',
    group: 'Действие',
    run: actions.openSettings
  })
  items.push({
    id: 'act:keygen',
    label: 'Генерация ключей',
    icon: '🔑',
    group: 'Действие',
    run: actions.openKeyGen
  })
  items.push({
    id: 'act:newserver',
    label: 'Добавить сервер',
    icon: '➕',
    group: 'Действие',
    run: actions.newServer
  })

  const activeTab = tabs.find((t) => t.key === activeKey)
  if (activeTab?.kind === 'terminal') {
    const sshLeaf = sshLeafForTools(activeTab)
    if (sshLeaf) {
      for (const x of WORKSPACE_TOOLS) {
        items.push({
          id: 'ws:' + x.id,
          label: x.label,
          hint: 'инструмент сервера',
          icon: '▣',
          group: 'Рабочее место',
          run: () => actions.setWorkspace(activeTab.key, x.id)
        })
      }
    }
  }

  for (const t of tabs) {
    items.push({
      id: 'tab:' + t.key,
      label: t.title,
      hint: 'перейти к вкладке',
      icon: t.kind === 'editor' ? '📝' : '🗔',
      group: 'Вкладка',
      run: () => actions.focusTab(t.key)
    })
  }
  return items
}
