import type { AppSettings } from '../shared/types'

/** Идентификаторы действий, на которые можно вешать горячие клавиши. */
export type ActionId =
  | 'command-palette'
  | 'new-terminal'
  | 'close-tab'
  | 'next-tab'
  | 'prev-tab'
  | 'split-right'
  | 'split-down'
  | 'close-pane'
  | 'focus-next-pane'
  | 'focus-prev-pane'
  | 'toggle-sftp'
  | 'toggle-broadcast'
  | 'open-settings'

export interface ActionDef {
  id: ActionId
  label: string
  /** Комбинация по умолчанию в каноничном виде (см. comboFromEvent). */
  default: string
}

/** Список действий + дефолтные комбинации. Порядок = порядок в настройках. */
export const ACTIONS: ActionDef[] = [
  { id: 'command-palette', label: 'Командная палитра / переход к серверу', default: 'Ctrl+Shift+P' },
  { id: 'new-terminal', label: 'Новый локальный терминал', default: 'Ctrl+Shift+T' },
  { id: 'close-tab', label: 'Закрыть вкладку', default: 'Ctrl+Shift+W' },
  { id: 'next-tab', label: 'Следующая вкладка', default: 'Ctrl+Tab' },
  { id: 'prev-tab', label: 'Предыдущая вкладка', default: 'Ctrl+Shift+Tab' },
  { id: 'split-right', label: 'Разделить вертикально (панели рядом)', default: 'Ctrl+Shift+E' },
  { id: 'split-down', label: 'Разделить горизонтально (друг над другом)', default: 'Ctrl+Shift+D' },
  { id: 'close-pane', label: 'Закрыть активную панель', default: 'Ctrl+Shift+X' },
  { id: 'focus-next-pane', label: 'Следующая панель (фокус)', default: 'Alt+]' },
  { id: 'focus-prev-pane', label: 'Предыдущая панель (фокус)', default: 'Alt+[' },
  { id: 'toggle-sftp', label: 'Файловый менеджер (SFTP)', default: 'Ctrl+Shift+F' },
  { id: 'toggle-broadcast', label: 'Broadcast-ввод', default: 'Ctrl+Shift+B' },
  { id: 'open-settings', label: 'Открыть настройки', default: 'Ctrl+,' }
]

const MODIFIER_KEYS = new Set(['Control', 'Alt', 'Shift', 'Meta'])

/** Подмножество полей KeyboardEvent — совместимо и с DOM-, и с React-событием. */
type KeyComboEvent = Pick<KeyboardEvent, 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey' | 'key'>

/** Превращает событие клавиатуры в каноничную строку, например "Ctrl+Shift+T". */
export function comboFromEvent(e: KeyComboEvent): string {
  if (MODIFIER_KEYS.has(e.key)) return '' // нажат только модификатор — ещё не комбинация
  const parts: string[] = []
  if (e.ctrlKey) parts.push('Ctrl')
  if (e.altKey) parts.push('Alt')
  if (e.shiftKey) parts.push('Shift')
  if (e.metaKey) parts.push('Meta')
  let key = e.key
  if (key === ' ') key = 'Space'
  else if (key.length === 1) key = key.toUpperCase()
  parts.push(key)
  return parts.join('+')
}

/** Итоговые привязки: дефолты, перекрытые пользовательскими настройками. */
export function resolveBindings(settings: AppSettings): Record<ActionId, string> {
  const out = {} as Record<ActionId, string>
  const overrides = settings.keybindings ?? {}
  for (const a of ACTIONS) out[a.id] = overrides[a.id] ?? a.default
  return out
}

/** Карта «комбинация → действие» для быстрого матчинга в обработчике. */
export function bindingLookup(settings: AppSettings): Map<string, ActionId> {
  const map = new Map<string, ActionId>()
  const resolved = resolveBindings(settings)
  for (const a of ACTIONS) {
    const combo = resolved[a.id]
    if (combo && !map.has(combo)) map.set(combo, a.id)
  }
  return map
}

/** Человекочитаемое отображение комбинации (для UI). */
export function formatCombo(combo: string): string {
  return combo || '—'
}
