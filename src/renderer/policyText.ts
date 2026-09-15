/**
 * Как назвать словами политику администратора.
 *
 * В политике - ключи `settings.json` и запреты, а человеку в окне настроек нужны те же
 * названия, что у переключателей: иначе «offline» ещё надо догадаться сопоставить с
 * «Закрытым контуром».
 */

import type { PolicyStatus } from '../shared/types'

const LABELS: Record<string, string> = {
  actionLog: 'журнал действий',
  actionLogSyslog: 'отправка журнала в syslog',
  offline: 'закрытый контур',
  closeToTray: 'сворачивание в трей',
  autoReconnect: 'авто-переподключение SSH',
  rdpCaptureShortcuts: 'перехват сочетаний в RDP',
  restoreTabsOnStart: 'восстановление вкладок',
  openLocalOnStart: 'локальный терминал при запуске',
  externalEditor: 'внешний редактор',
  sftpConcurrency: 'параллельные SFTP-передачи',
  theme: 'цветовая схема',
  fontSize: 'размер шрифта',
  fontFamily: 'шрифт',
  keybindings: 'горячие клавиши'
}

export function settingLabel(key: string): string {
  return LABELS[key] ?? key
}

/** Список заданного политикой: названия через запятую. */
export function lockedText(keys: string[]): string {
  return keys.map(settingLabel).join(', ')
}

/** Что действует по политике - по пункту на ограничение, словами. */
export function policySummary(p: PolicyStatus): string[] {
  const out: string[] = []
  if (p.locked.length > 0) out.push(`заданы настройки: ${lockedText(p.locked)}`)
  if (p.allowedHosts) {
    out.push(
      p.allowedHosts.length > 0
        ? `подключаться можно только к: ${p.allowedHosts.join(', ')}`
        : 'подключаться нельзя ни к одному серверу'
    )
  }
  if (p.forbidLegacySshAlgorithms) out.push('устаревшие алгоритмы SSH запрещены')
  if (p.forbidSavedPasswords) out.push('пароли не сохраняются и спрашиваются при подключении')
  if (p.requireMasterPassword) out.push('мастер-пароль обязателен')
  if (p.forbidLocalTerminal) out.push('локальный терминал запрещён')
  if (p.forbidSessionRecording) out.push('запись сессии в файл запрещена')
  return out
}
