import { createRoot } from 'react-dom/client'
import { Gate } from './Gate'
import { SettingsProvider } from './SettingsContext'
import { DockerLogsWindow } from './components/dockerLogs'
import { SftpWindow } from './components/SftpWindow'
import { WorkspaceWindow } from './components/workspaceWindow'
import { DetachedTabWindow } from './components/DetachedTabWindow'
import { api } from '../api'
import { checkForUpdatesOnStartup } from './updater'
import '@xterm/xterm/css/xterm.css'
import './styles.css'

window.api = api

/**
 * Гасим меню WebView2 («Назад», «Обновить», «Печать», «Проверить») — в десктопном
 * приложении оно неуместно. Наши собственные меню вызывают `stopPropagation`,
 * поэтому до этого обработчика не доходят и продолжают работать.
 */
document.addEventListener('contextmenu', (e) => e.preventDefault())

const q = new URLSearchParams(window.location.search)
const detachedLogs = q.get('dockerLogs') === '1'
const detachedSftp = q.get('sftp') === '1'
const detachedWorkspace = q.get('workspace') === '1'
const detachedTab = q.get('detachedTab') === '1'

// Каждое окно — свой процесс рендера, поэтому провайдер настроек нужен в каждом,
// иначе открепленные панели берут DEFAULT_SETTINGS вместо сохранённых.
createRoot(document.getElementById('root')!).render(
  <SettingsProvider>
    {detachedLogs ? (
      <DockerLogsWindow />
    ) : detachedSftp ? (
      <SftpWindow />
    ) : detachedWorkspace ? (
      <WorkspaceWindow />
    ) : detachedTab ? (
      <DetachedTabWindow />
    ) : (
      <Gate />
    )}
  </SettingsProvider>
)

// Только в главном окне: иначе четыре открепленных спросят про обновление четыре раза.
if (!detachedLogs && !detachedSftp && !detachedWorkspace && !detachedTab) {
  checkForUpdatesOnStartup()
}
