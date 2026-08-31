import type { PaneLeaf } from './paneTree'
import { allLeaves } from './paneTree'
import type { Tab } from './tabs'

/**
 * Сводный статус сервера для боковой панели.
 *
 * Один сервер может быть открыт в нескольких вкладках и панелях сразу, и статусы у них
 * разные: в одной подключено, в другой идёт переподключение, в третьей ошибка. Точка
 * рядом с именем сервера одна — значит нужно выбрать.
 *
 * Выбираем **лучший**, а не худший, и это осознанно: точка отвечает на вопрос «есть ли у
 * меня рабочее соединение с этим сервером». Если хоть одна панель подключена — есть, и
 * красная точка из-за соседней сорвавшейся вкладки только сбивала бы с толку.
 */
const RANK = { connected: 4, reconnecting: 3, connecting: 2, error: 1 } as const

export type ShownStatus = keyof typeof RANK

/**
 * Статусы, которые показываются точкой.
 *
 * Остальные (`closed`, отсутствие статуса) намеренно не показываются вовсе: закрытая
 * панель — это не состояние сервера, а её отсутствие.
 */
function isShown(status: PaneLeaf['status'] | undefined): status is ShownStatus {
  return !!status && status in RANK
}

export function aggregateServerStatuses(tabs: Tab[]): Record<string, ShownStatus> {
  const out: Record<string, ShownStatus> = {}
  for (const tab of tabs) {
    for (const leaf of allLeaves(tab.root)) {
      if (leaf.kind !== 'ssh' || !leaf.serverId) continue
      if (!isShown(leaf.status)) continue
      const current = out[leaf.serverId]
      if (!current || RANK[leaf.status] > RANK[current]) out[leaf.serverId] = leaf.status
    }
  }
  return out
}
