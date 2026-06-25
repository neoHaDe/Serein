import { useState } from 'react'
import type { GeneratedKey, KeyType } from '../../shared/types'

interface ConnectedSession {
  sessionId: string
  title: string
}

interface Props {
  connectedSessions: ConnectedSession[]
  onClose: () => void
}

export function KeyGenModal({ connectedSessions, onClose }: Props): JSX.Element {
  const [type, setType] = useState<KeyType>('ed25519')
  const [bits, setBits] = useState(4096)
  const [comment, setComment] = useState('')
  const [passphrase, setPassphrase] = useState('')
  const [busy, setBusy] = useState(false)
  const [key, setKey] = useState<GeneratedKey | null>(null)
  const [msg, setMsg] = useState<string | null>(null)
  const [installTarget, setInstallTarget] = useState('')

  const generate = async (): Promise<void> => {
    setBusy(true)
    setMsg(null)
    try {
      const result = await window.api.keygen.generate({
        type,
        bits: type === 'rsa' ? bits : undefined,
        comment: comment.trim() || undefined,
        passphrase: passphrase || undefined
      })
      setKey(result)
    } catch (e) {
      setMsg('Ошибка генерации: ' + (e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  const copyPublic = async (): Promise<void> => {
    if (!key) return
    await navigator.clipboard.writeText(key.publicKey)
    setMsg('Публичный ключ скопирован в буфер обмена.')
  }

  const save = async (): Promise<void> => {
    if (!key) return
    const defaultName = type === 'ed25519' ? 'id_ed25519' : 'id_rsa'
    const res = await window.api.keygen.save(key, defaultName)
    if (res.saved) setMsg(`Сохранено: ${res.privatePath} и ${res.publicPath}`)
  }

  const install = async (): Promise<void> => {
    if (!key || !installTarget) return
    setBusy(true)
    setMsg(null)
    try {
      await window.api.keygen.install(installTarget, key.publicKey)
      const t = connectedSessions.find((s) => s.sessionId === installTarget)
      setMsg(`Публичный ключ установлен на «${t?.title ?? 'сервер'}».`)
    } catch (e) {
      setMsg('Ошибка установки: ' + (e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Генерация SSH-ключа</h2>

        <div className="row">
          <label style={{ flex: 1 }}>
            Тип ключа
            <select value={type} onChange={(e) => setType(e.target.value as KeyType)}>
              <option value="ed25519">Ed25519 (рекомендуется)</option>
              <option value="rsa">RSA</option>
            </select>
          </label>
          {type === 'rsa' && (
            <label style={{ flex: 1 }}>
              Длина (бит)
              <select value={bits} onChange={(e) => setBits(Number(e.target.value))}>
                <option value={2048}>2048</option>
                <option value={4096}>4096</option>
              </select>
            </label>
          )}
        </div>

        <label>
          Комментарий (метка ключа)
          <input value={comment} onChange={(e) => setComment(e.target.value)} placeholder="user@machine" />
        </label>

        <label>
          Парольная фраза (необязательно)
          <input
            type="password"
            value={passphrase}
            onChange={(e) => setPassphrase(e.target.value)}
            placeholder="Шифрует приватный ключ"
          />
        </label>

        <button className="primary" style={{ width: '100%' }} disabled={busy} onClick={generate}>
          {busy ? 'Генерация…' : key ? 'Сгенерировать заново' : 'Сгенерировать'}
        </button>

        {key && (
          <div className="keygen-result">
            <label style={{ marginTop: 14 }}>
              Публичный ключ
              <textarea className="keygen-pubkey" readOnly value={key.publicKey} rows={3} />
            </label>
            <div className="keygen-actions">
              <button className="secondary" onClick={copyPublic}>📋 Копировать</button>
              <button className="secondary" onClick={save}>💾 Сохранить на диск</button>
            </div>

            <div className="keygen-install">
              <label>
                Установить на подключённый сервер (ssh-copy-id)
                <select value={installTarget} onChange={(e) => setInstallTarget(e.target.value)}>
                  <option value="">— Выберите сессию —</option>
                  {connectedSessions.map((s) => (
                    <option key={s.sessionId} value={s.sessionId}>
                      {s.title}
                    </option>
                  ))}
                </select>
              </label>
              <button className="primary" disabled={!installTarget || busy} onClick={install}>
                Установить ключ
              </button>
              {connectedSessions.length === 0 && (
                <div className="hint" style={{ padding: '4px 0 0' }}>
                  Нет активных SSH-сессий. Подключитесь к серверу, чтобы установить ключ.
                </div>
              )}
            </div>
          </div>
        )}

        {msg && <div className="keygen-msg">{msg}</div>}

        <div className="modal-actions">
          <button className="secondary" onClick={onClose}>Закрыть</button>
        </div>
      </div>
    </div>
  )
}
