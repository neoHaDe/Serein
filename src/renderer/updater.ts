import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { ask, message } from '@tauri-apps/plugin-dialog'

// Проверка обновлений через endpoint из tauri.conf (nehade.xyz).
// silent=true — молчать, если обновлений нет или сеть недоступна (авто-проверка при старте).
// silent=false — показывать результат всегда (ручная кнопка в настройках).
export async function checkForUpdates(silent: boolean): Promise<void> {
  let update
  try {
    update = await check()
  } catch (e) {
    if (!silent) {
      await message('Не удалось проверить обновления: ' + (e as Error).message, {
        title: 'Обновление',
        kind: 'error'
      })
    }
    return
  }

  if (!update) {
    if (!silent) {
      await message('У вас последняя версия Serein.', { title: 'Обновление' })
    }
    return
  }

  const yes = await ask(
    `Доступна версия ${update.version} (у вас ${update.currentVersion}).` +
      (update.body ? `\n\n${update.body}` : '') +
      '\n\nСкачать, установить и перезапустить приложение?',
    { title: 'Доступно обновление', kind: 'info' }
  )
  if (!yes) return

  try {
    await update.downloadAndInstall()
    await relaunch()
  } catch (e) {
    await message('Ошибка установки обновления: ' + (e as Error).message, {
      title: 'Обновление',
      kind: 'error'
    })
  }
}
