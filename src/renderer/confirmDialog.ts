import { ask } from '@tauri-apps/plugin-dialog'

/**
 * Вопрос перед необратимым действием. Ответ надо дождаться: `await confirmAction(...)`.
 *
 * `window.confirm` для этого не годится. Плагин диалогов Tauri подменяет его асинхронной
 * функцией, и `if (!confirm(...)) return` получал обещание вместо ответа: оно всегда
 * истинно, поэтому удаление, `DELETE` без `WHERE` и перезапись чужой правки выполнялись
 * без вопроса. В плагине 2.7.2 подмена к тому же зовёт команду, которой больше нет, и окна
 * не показывает вовсе. Правило линтера не даёт вернуть `confirm`.
 *
 * Если окно показать не удалось, ответ - «нет»: не выполнить действие безопаснее, чем
 * выполнить его, не спросив.
 */
export async function confirmAction(message: string): Promise<boolean> {
  try {
    return await ask(message, { title: 'Serein', kind: 'warning', okLabel: 'Да', cancelLabel: 'Отмена' })
  } catch {
    return false
  }
}
