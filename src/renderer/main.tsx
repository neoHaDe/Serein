import { createRoot } from 'react-dom/client'
import { Gate } from './Gate'
import { SettingsProvider } from './SettingsContext'
import { api } from '../api'
import '@xterm/xterm/css/xterm.css'
import './styles.css'

// Мост к Rust-бэкенду выставляем как window.api — renderer ждёт его как в Electron.
window.api = api

// StrictMode намеренно отключён: двойной вызов эффектов в dev приводил бы
// к открытию лишних SSH-сессий и PTY. Терминалы должны создаваться ровно один раз.
createRoot(document.getElementById('root')!).render(
  <SettingsProvider>
    <Gate />
  </SettingsProvider>
)
