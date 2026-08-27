import { useEffect, useState, type ReactNode } from 'react'
import App from './App'
import { UnlockScreen } from './components/UnlockScreen'
import { AppChrome } from './components/WindowChrome'

function Shell({ children }: { children: ReactNode }): JSX.Element {
  return (
    <div className="app-shell">
      <AppChrome title="Serein" />
      <div className="app-shell-body">{children}</div>
    </div>
  )
}

/** Решает, показать ли экран разблокировки (мастер-пароль) перед приложением. */
export function Gate(): JSX.Element {
  const [state, setState] = useState<'loading' | 'locked' | 'open'>('loading')

  useEffect(() => {
    document.documentElement.classList.add('frameless')
    window.api.vault.status().then((s) => setState(s.locked ? 'locked' : 'open'))
  }, [])

  if (state === 'loading') {
    return (
      <Shell>
        <div className="unlock-screen" />
      </Shell>
    )
  }
  if (state === 'locked') {
    return (
      <Shell>
        <UnlockScreen onUnlocked={() => setState('open')} />
      </Shell>
    )
  }
  return (
    <Shell>
      <App />
    </Shell>
  )
}
