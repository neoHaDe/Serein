/**
 * Как назвать словами настройки, заданные политикой администратора.
 *
 * В политике - ключи `settings.json`, а человеку в окне настроек нужны те же названия, что
 * у переключателей: иначе «offline» ещё надо догадаться сопоставить с «Закрытым контуром».
 */

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

/** Список заданного политикой для плашки: названия через запятую. */
export function lockedText(keys: string[]): string {
  return keys.map(settingLabel).join(', ')
}
