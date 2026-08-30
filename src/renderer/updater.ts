import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { ask, message } from '@tauri-apps/plugin-dialog'
import { errText } from './errText'
import { installKind } from './platform'

const RELEASES_URL = 'https://github.com/neoHaDe/Serein/releases/latest'

/**
 * Паузы перед попытками тихой проверки при запуске.
 *
 * Одной попытки мало: приложение часто стартует раньше, чем поднимается сеть —
 * автозапуск при входе в систему, открытие крышки ноутбука, подключение к Wi-Fi.
 * Тогда единственная проверка сорвалась бы молча, и про обновление узнали бы
 * только при следующем запуске. Повторяем, но лишь при ошибке связи: как только
 * ответ получен — неважно, есть обновление или нет, — попытки прекращаются.
 */
const STARTUP_DELAYS = [3_000, 30_000, 180_000]

/** Предложить обновление с учётом того, как приложение установлено. */
async function offerUpdate(update: Update): Promise<void> {
  const head =
    `Доступна версия ${update.version} (у вас ${update.currentVersion}).` +
    (update.body ? `\n\n${update.body}` : '')

  if ((await installKind()) === 'package') {
    // Из `.deb` бинарь лежит в /usr/bin и принадлежит менеджеру пакетов: заменить
    // его на месте нельзя. Раньше апдейтер честно предлагал установку и падал уже
    // на скачивании — обещание, которое некому выполнить.
    const copy = await ask(
      head +
        '\n\nSerein установлен из пакета, поэтому обновиться на месте нельзя — ' +
        'скачайте новый .deb со страницы релизов.\n\nСкопировать ссылку?',
      { title: 'Доступно обновление', kind: 'info' }
    )
    if (copy) await window.api.clipboard.write(RELEASES_URL)
    return
  }

  const yes = await ask(head + '\n\nСкачать, установить и перезапустить приложение?', {
    title: 'Доступно обновление',
    kind: 'info'
  })
  if (!yes) return

  try {
    await update.downloadAndInstall()
    await relaunch()
  } catch (e) {
    await message('Ошибка установки обновления: ' + errText(e), {
      title: 'Обновление',
      kind: 'error'
    })
  }
}

/**
 * Проверка обновлений через endpoint из `tauri.conf` (nehade.xyz).
 *
 * `silent=true` — молчать, если обновлений нет или сеть недоступна.
 * `silent=false` — показывать результат всегда (кнопка в настройках).
 */
export async function checkForUpdates(silent: boolean): Promise<void> {
  let update: Update | null
  try {
    update = await check()
  } catch (e) {
    if (!silent) {
      await message('Не удалось проверить обновления: ' + errText(e), {
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

  await offerUpdate(update)
}

/**
 * Тихая проверка при запуске: молчит, когда обновления нет, и спрашивает, когда есть.
 * При обрыве связи повторяет по расписанию `STARTUP_DELAYS`, а не сдаётся с первой попытки.
 */
export function checkForUpdatesOnStartup(): void {
  let attempt = 0

  const tick = async (): Promise<void> => {
    try {
      const update = await check()
      // Ответ получен — на этом запуске больше не тревожим сеть.
      if (update) await offerUpdate(update)
    } catch {
      attempt += 1
      if (attempt < STARTUP_DELAYS.length) {
        window.setTimeout(() => void tick(), STARTUP_DELAYS[attempt])
      }
    }
  }

  window.setTimeout(() => void tick(), STARTUP_DELAYS[0])
}
