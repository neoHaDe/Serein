import { useState } from 'react'

export function UnlockScreen({ onUnlocked }: { onUnlocked: () => void }): JSX.Element {
  const [password, setPassword] = useState('')
  const [error, setError] = useState(false)
  const [busy, setBusy] = useState(false)

  const submit = async (): Promise<void> => {
    if (!password) return
    setBusy(true)
    setError(false)
    const ok = await window.api.vault.unlock(password)
    setBusy(false)
    if (ok) onUnlocked()
    else {
      setError(true)
      setPassword('')
    }
  }

  return (
    <div className="unlock-screen">
      <div className="unlock-box">
        <div className="unlock-logo">⌁ TermiNAL</div>
        <p>Введите мастер-пароль для разблокировки</p>
        <input
          type="password"
          autoFocus
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && submit()}
          className={error ? 'error' : ''}
          placeholder="Мастер-пароль"
        />
        {error && <div className="unlock-error">Неверный пароль</div>}
        <button className="primary" disabled={busy} onClick={submit}>
          {busy ? 'Проверка…' : 'Разблокировать'}
        </button>
      </div>
    </div>
  )
}
