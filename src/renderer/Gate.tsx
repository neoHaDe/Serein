import { useEffect, useState } from 'react'
import App from './App'
import { UnlockScreen } from './components/UnlockScreen'

/** Решает, показать ли экран разблокировки (мастер-пароль) перед приложением. */
export function Gate(): JSX.Element {
  const [state, setState] = useState<'loading' | 'locked' | 'open'>('loading')

  useEffect(() => {
    window.api.vault.status().then((s) => setState(s.locked ? 'locked' : 'open'))
  }, [])

  if (state === 'loading') return <div className="unlock-screen" />
  if (state === 'locked') return <UnlockScreen onUnlocked={() => setState('open')} />
  return <App />
}
