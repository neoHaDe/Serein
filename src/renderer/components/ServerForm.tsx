import { useState } from 'react'
import type { AuthType, ServerConfig, TunnelConfig, TunnelType } from '../../shared/types'

interface Props {
  initial: ServerConfig | null // null = создание нового
  servers: ServerConfig[]
  onCancel: () => void
  onSave: (cfg: ServerConfig) => void
}

const COLORS = ['#7aa2f7', '#9ece6a', '#e0af68', '#f7768e', '#bb9af7', '#7dcfff']

function tunnelDesc(t: TunnelConfig): string {
  if (t.type === 'local') return `127.0.0.1:${t.localPort} → ${t.remoteHost}:${t.remotePort}`
  if (t.type === 'remote') return `сервер:${t.remotePort} → 127.0.0.1:${t.localPort}`
  return `SOCKS5 127.0.0.1:${t.localPort}`
}

function AddTunnelForm({
  onAdd,
  onCancel
}: {
  onAdd: (t: TunnelConfig) => void
  onCancel: () => void
}): JSX.Element {
  const [type, setType] = useState<TunnelType>('local')
  const [localPort, setLocalPort] = useState('')
  const [remoteHost, setRemoteHost] = useState('')
  const [remotePort, setRemotePort] = useState('')

  const submit = (): void => {
    const lp = parseInt(localPort)
    if (!lp || lp < 1 || lp > 65535) {
      alert('Укажите корректный локальный порт (1–65535)')
      return
    }
    if (type !== 'dynamic') {
      const rp = parseInt(remotePort)
      if (!rp || rp < 1 || rp > 65535) {
        alert('Укажите корректный удалённый порт (1–65535)')
        return
      }
      if (type === 'local' && !remoteHost.trim()) {
        alert('Укажите удалённый хост')
        return
      }
    }
    onAdd({
      id: crypto.randomUUID(),
      type,
      localPort: lp,
      remoteHost: type === 'local' ? remoteHost.trim() || 'localhost' : undefined,
      remotePort: type !== 'dynamic' ? parseInt(remotePort) : undefined
    })
  }

  return (
    <div className="add-tunnel-form">
      <div className="row">
        <label style={{ flex: 2 }}>
          Тип
          <select value={type} onChange={(e) => setType(e.target.value as TunnelType)}>
            <option value="local">Local (-L) — локал → удалённый</option>
            <option value="remote">Remote (-R) — сервер → локал</option>
            <option value="dynamic">Dynamic SOCKS5 (-D)</option>
          </select>
        </label>
        <label style={{ flex: 1 }}>
          Локальный порт
          <input type="number" value={localPort} onChange={(e) => setLocalPort(e.target.value)} placeholder="8080" />
        </label>
      </div>
      {type !== 'dynamic' && (
        <div className="row">
          {type === 'local' && (
            <label style={{ flex: 2 }}>
              Удалённый хост
              <input
                value={remoteHost}
                onChange={(e) => setRemoteHost(e.target.value)}
                placeholder="localhost"
              />
            </label>
          )}
          <label style={{ flex: 1 }}>
            {type === 'local' ? 'Порт на сервере' : 'Порт на сервере'}
            <input
              type="number"
              value={remotePort}
              onChange={(e) => setRemotePort(e.target.value)}
              placeholder="5432"
            />
          </label>
        </div>
      )}
      <div className="modal-actions" style={{ marginTop: 6 }}>
        <button className="secondary" onClick={onCancel}>Отмена</button>
        <button className="primary" onClick={submit}>Добавить</button>
      </div>
    </div>
  )
}

export function ServerForm({ initial, servers, onCancel, onSave }: Props): JSX.Element {
  const isEdit = !!initial
  const [name, setName] = useState(initial?.name ?? '')
  const [host, setHost] = useState(initial?.host ?? '')
  const [port, setPort] = useState(initial?.port ?? 22)
  const [username, setUsername] = useState(initial?.username ?? 'root')
  const [authType, setAuthType] = useState<AuthType>(initial?.authType ?? 'password')
  const [password, setPassword] = useState('')
  const [privateKeyPath, setPrivateKeyPath] = useState(initial?.privateKeyPath ?? '')
  const [passphrase, setPassphrase] = useState('')
  const [group, setGroup] = useState(initial?.group ?? '')
  const [color, setColor] = useState(initial?.color ?? COLORS[0])
  const [proxyJump, setProxyJump] = useState(initial?.proxyJump ?? '')
  const [tunnels, setTunnels] = useState<TunnelConfig[]>(initial?.tunnels ?? [])
  const [addingTunnel, setAddingTunnel] = useState(false)
  const [executeOnConnect, setExecuteOnConnect] = useState(initial?.executeOnConnect ?? '')
  const [agentForward, setAgentForward] = useState(initial?.agentForward ?? false)

  // Кандидаты в jump-хосты: любой сервер, кроме редактируемого (защита от прямого self-ref).
  const jumpCandidates = servers.filter((s) => s.id !== initial?.id)

  const submit = (): void => {
    if (!name.trim() || !host.trim() || !username.trim()) {
      alert('Заполните название, хост и пользователя')
      return
    }
    const cfg: ServerConfig = {
      id: initial?.id ?? '',
      name: name.trim(),
      host: host.trim(),
      port: Number(port) || 22,
      username: username.trim(),
      authType,
      group: group.trim() || undefined,
      color,
      proxyJump: proxyJump || undefined,
      tunnels: tunnels.length ? tunnels : undefined,
      executeOnConnect: executeOnConnect.trim() || undefined,
      agentForward: agentForward || undefined,
      privateKeyPath: authType === 'key' ? privateKeyPath || undefined : undefined,
      // Пустое поле секрета => undefined => существующее значение не трогаем.
      password: authType === 'password' && password ? password : undefined,
      passphrase: authType === 'key' && passphrase ? passphrase : undefined
    }
    onSave(cfg)
  }

  const pickKey = async (): Promise<void> => {
    const p = await window.api.dialog.pickKey()
    if (p) setPrivateKeyPath(p)
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{isEdit ? 'Изменить сервер' : 'Новый сервер'}</h2>

        <label>
          Название
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Прод-сервер" autoFocus />
        </label>

        <div className="row">
          <label style={{ flex: 3 }}>
            Хост
            <input value={host} onChange={(e) => setHost(e.target.value)} placeholder="192.168.1.10 / example.com" />
          </label>
          <label style={{ flex: 1 }}>
            Порт
            <input type="number" value={port} onChange={(e) => setPort(Number(e.target.value))} />
          </label>
        </div>

        <div className="row">
          <label style={{ flex: 2 }}>
            Пользователь
            <input value={username} onChange={(e) => setUsername(e.target.value)} />
          </label>
          <label style={{ flex: 2 }}>
            Группа
            <input value={group} onChange={(e) => setGroup(e.target.value)} placeholder="Продакшен" />
          </label>
        </div>

        <label>
          Аутентификация
          <select value={authType} onChange={(e) => setAuthType(e.target.value as AuthType)}>
            <option value="password">Пароль</option>
            <option value="key">Приватный ключ</option>
            <option value="agent">SSH-агент</option>
          </select>
        </label>

        {authType === 'password' && (
          <label>
            Пароль
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={isEdit ? '•••••• (оставьте пустым, чтобы не менять)' : ''}
            />
          </label>
        )}

        {authType === 'key' && (
          <>
            <label>
              Файл приватного ключа
              <div className="row">
                <input value={privateKeyPath} onChange={(e) => setPrivateKeyPath(e.target.value)} placeholder="C:\Users\you\.ssh\id_ed25519" />
                <button className="secondary" onClick={pickKey}>
                  Обзор…
                </button>
              </div>
            </label>
            <label>
              Парольная фраза ключа (если есть)
              <input
                type="password"
                value={passphrase}
                onChange={(e) => setPassphrase(e.target.value)}
                placeholder={isEdit ? '•••••• (оставьте пустым, чтобы не менять)' : ''}
              />
            </label>
          </>
        )}

        <label>
          Подключаться через (jump host / бастион)
          <select value={proxyJump} onChange={(e) => setProxyJump(e.target.value)}>
            <option value="">— Прямое подключение —</option>
            {jumpCandidates.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name} ({s.username}@{s.host})
              </option>
            ))}
          </select>
        </label>

        <label>
          Команда при подключении (выполнится после открытия shell)
          <input
            value={executeOnConnect}
            onChange={(e) => setExecuteOnConnect(e.target.value)}
            placeholder="cd /var/www && tmux attach || tmux"
          />
        </label>

        {authType === 'agent' && (
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={agentForward}
              onChange={(e) => setAgentForward(e.target.checked)}
            />
            Пробрасывать SSH-агент на сервер (agent forwarding)
          </label>
        )}

        <div className="tunnel-section">
          <div className="tunnel-section-header">
            <span>Туннели при подключении</span>
            {!addingTunnel && (
              <button className="mini" onClick={() => setAddingTunnel(true)}>+ Добавить</button>
            )}
          </div>
          {tunnels.map((t) => (
            <div key={t.id} className="tunnel-config-row">
              <span className="tunnel-type-badge">{t.type === 'local' ? 'L' : t.type === 'remote' ? 'R' : 'D'}</span>
              <span className="tunnel-config-desc">{tunnelDesc(t)}</span>
              <button className="mini danger" onClick={() => setTunnels((prev) => prev.filter((x) => x.id !== t.id))}>✕</button>
            </div>
          ))}
          {addingTunnel && (
            <AddTunnelForm
              onAdd={(t) => {
                setTunnels((prev) => [...prev, t])
                setAddingTunnel(false)
              }}
              onCancel={() => setAddingTunnel(false)}
            />
          )}
        </div>

        <label>
          Цвет
          <div className="colors">
            {COLORS.map((c) => (
              <button
                key={c}
                className={'color-swatch' + (c === color ? ' selected' : '')}
                style={{ background: c }}
                onClick={() => setColor(c)}
              />
            ))}
          </div>
        </label>

        <div className="modal-actions">
          <button className="secondary" onClick={onCancel}>
            Отмена
          </button>
          <button className="primary" onClick={submit}>
            Сохранить
          </button>
        </div>
      </div>
    </div>
  )
}
