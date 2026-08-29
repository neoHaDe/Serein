import { useEffect, useState } from 'react'
import type {
  AgentIdentity,
  AuthType,
  SerialConfig,
  SerialPortInfo,
  ServerConfig,
  TunnelConfig,
  TunnelType
} from '../../shared/types'
import { errText } from '../errText'

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

/** Порт по умолчанию для типа подключения. У сырого TCP общепринятого нет. */
function defaultPort(kind: ServerConfig['connection']): number {
  return kind === 'telnet' ? 23 : kind === 'raw' ? 0 : 22
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
  const [connection, setConnection] = useState<ServerConfig['connection']>(
    initial?.connection ?? 'ssh'
  )
  const [telnetEol, setTelnetEol] = useState<'crlf' | 'cr-nul' | 'cr'>(initial?.telnetEol ?? 'crlf')
  /** telnet и сырой TCP: адрес и порт есть, пользователя и аутентификации нет. */
  const isTcp = connection === 'telnet' || connection === 'raw'
  const [proxyCommand, setProxyCommand] = useState(initial?.proxyCommand ?? '')
  const [sshCompression, setSshCompression] = useState(initial?.sshCompression ?? false)
  const [sshLegacyAlgos, setSshLegacyAlgos] = useState(initial?.sshLegacyAlgos ?? false)
  const [serial, setSerial] = useState<SerialConfig>(
    initial?.serial ?? {
      port: '',
      baudRate: 115200,
      dataBits: 8,
      stopBits: 1,
      parity: 'none',
      flowControl: 'none'
    }
  )
  const [comPorts, setComPorts] = useState<SerialPortInfo[]>([])
  const patchSerial = (patch: Partial<SerialConfig>): void => setSerial((s) => ({ ...s, ...patch }))
  const [agentForward, setAgentForward] = useState(initial?.agentForward ?? false)
  const [agentKey, setAgentKey] = useState(initial?.agentKey ?? '')
  const [agentKeys, setAgentKeys] = useState<AgentIdentity[]>([])
  const [agentError, setAgentError] = useState('')
  const [agentLoading, setAgentLoading] = useState(false)

  // Кандидаты в jump-хосты: любой сервер, кроме редактируемого (защита от прямого self-ref).
  const jumpCandidates = servers.filter((s) => s.id !== initial?.id)

  const loadAgentKeys = async (): Promise<void> => {
    setAgentLoading(true)
    try {
      const r = await window.api.agent.identities()
      setAgentKeys(r.keys ?? [])
      setAgentError(r.ok ? '' : r.error ?? 'SSH-агент недоступен')
    } catch (e) {
      setAgentKeys([])
      setAgentError(errText(e))
    } finally {
      setAgentLoading(false)
    }
  }

  // Список тянем только когда он реально нужен — при выборе аутентификации через агент.
  useEffect(() => {
    if (authType === 'agent') void loadAgentKeys()
  }, [authType])

  const loadComPorts = async (): Promise<void> => {
    try {
      setComPorts(await window.api.serial.ports())
    } catch {
      setComPorts([])
    }
  }

  useEffect(() => {
    if (connection === 'serial') void loadComPorts()
  }, [connection])

  const submit = (): void => {
    if (!name.trim()) {
      alert('Заполните название')
      return
    }
    // У COM-порта нет ни хоста, ни пользователя — требуем только имя порта.
    if (connection === 'serial') {
      if (!serial.port.trim()) {
        alert('Выберите COM-порт')
        return
      }
    } else if (isTcp) {
      // У telnet и сырого TCP пользователя нет: логин спрашивает сама железка,
      // если ей это вообще нужно.
      if (!host.trim()) {
        alert('Укажите адрес хоста')
        return
      }
      if (!port || port < 1 || port > 65535) {
        alert('Укажите порт от 1 до 65535')
        return
      }
    } else if (!host.trim() || !username.trim()) {
      alert('Заполните хост и пользователя')
      return
    }
    const cfg: ServerConfig = {
      id: initial?.id ?? '',
      name: name.trim(),
      // Поля SSH сохраняем как есть: если профиль переключат обратно, настройки не пропадут.
      host: host.trim(),
      port: Number(port) || defaultPort(connection),
      username: username.trim() || 'root',
      authType,
      connection,
      serial: connection === 'serial' ? { ...serial, port: serial.port.trim() } : initial?.serial,
      telnetEol: connection === 'telnet' ? telnetEol : initial?.telnetEol,
      proxyCommand: connection === 'ssh' && proxyCommand.trim() ? proxyCommand.trim() : undefined,
      sshCompression: connection === 'ssh' && sshCompression ? true : undefined,
      sshLegacyAlgos: connection === 'ssh' && sshLegacyAlgos ? true : undefined,
      group: group.trim() || undefined,
      color,
      proxyJump: proxyJump || undefined,
      tunnels: tunnels.length ? tunnels : undefined,
      executeOnConnect: executeOnConnect.trim() || undefined,
      agentForward: agentForward || undefined,
      agentKey: authType === 'agent' && agentKey ? agentKey : undefined,
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

        <label>
          Тип подключения
          <select
            value={connection}
            onChange={(e) => {
              const next = e.target.value as ServerConfig['connection']
              // Порт подставляем только если он ещё стандартный для прежнего типа:
              // вручную выставленный 2001 у консольного сервера затирать нельзя.
              if (port === defaultPort(connection)) setPort(defaultPort(next))
              setConnection(next)
            }}
          >
            <option value="ssh">SSH</option>
            <option value="serial">Последовательный порт (COM)</option>
            <option value="telnet">Telnet</option>
            <option value="raw">TCP без обработки</option>
          </select>
        </label>

        {connection === 'serial' ? (
          <>
            <div className="row">
              <label style={{ flex: 3 }}>
                <span className="agent-label">
                  COM-порт
                  <button type="button" className="mini" onClick={() => void loadComPorts()}>
                    Обновить
                  </button>
                </span>
                <select value={serial.port} onChange={(e) => patchSerial({ port: e.target.value })}>
                  <option value="">— Выберите порт —</option>
                  {comPorts.map((p) => (
                    <option key={p.port} value={p.port}>
                      {p.port}
                      {p.label ? ` · ${p.label}` : ''}
                    </option>
                  ))}
                  {/* Порт мог быть сохранён при воткнутом переходнике, а сейчас его нет. */}
                  {serial.port && !comPorts.some((p) => p.port === serial.port) && (
                    <option value={serial.port}>{serial.port} (сейчас не подключён)</option>
                  )}
                </select>
              </label>
              <label style={{ flex: 2 }}>
                Скорость, бод
                <input
                  type="number"
                  value={serial.baudRate}
                  onChange={(e) => patchSerial({ baudRate: Number(e.target.value) || 115200 })}
                  list="baud-presets"
                />
                <datalist id="baud-presets">
                  {[9600, 19200, 38400, 57600, 115200, 230400, 921600].map((b) => (
                    <option key={b} value={b} />
                  ))}
                </datalist>
              </label>
            </div>

            {comPorts.length === 0 && (
              <div className="agent-hint">
                Портов не найдено. Воткните переходник USB-UART или проверьте драйвер.
              </div>
            )}

            <div className="row">
              <label style={{ flex: 1 }}>
                Биты данных
                <select
                  value={serial.dataBits ?? 8}
                  onChange={(e) => patchSerial({ dataBits: Number(e.target.value) as 5 | 6 | 7 | 8 })}
                >
                  {[8, 7, 6, 5].map((n) => (
                    <option key={n} value={n}>{n}</option>
                  ))}
                </select>
              </label>
              <label style={{ flex: 1 }}>
                Стоп-биты
                <select
                  value={serial.stopBits ?? 1}
                  onChange={(e) => patchSerial({ stopBits: Number(e.target.value) as 1 | 2 })}
                >
                  <option value={1}>1</option>
                  <option value={2}>2</option>
                </select>
              </label>
              <label style={{ flex: 1 }}>
                Чётность
                <select
                  value={serial.parity ?? 'none'}
                  onChange={(e) =>
                    patchSerial({ parity: e.target.value as 'none' | 'odd' | 'even' })
                  }
                >
                  <option value="none">Нет</option>
                  <option value="even">Чётная</option>
                  <option value="odd">Нечётная</option>
                </select>
              </label>
              <label style={{ flex: 1 }}>
                Управление потоком
                <select
                  value={serial.flowControl ?? 'none'}
                  onChange={(e) =>
                    patchSerial({
                      flowControl: e.target.value as 'none' | 'software' | 'hardware'
                    })
                  }
                >
                  <option value="none">Нет</option>
                  <option value="hardware">RTS/CTS</option>
                  <option value="software">XON/XOFF</option>
                </select>
              </label>
            </div>

            <div className="row">
              <label className="checkbox-row" style={{ flex: 1 }}>
                <input
                  type="checkbox"
                  checked={serial.dtr ?? false}
                  onChange={(e) => patchSerial({ dtr: e.target.checked })}
                />
                Поднять DTR
              </label>
              <label className="checkbox-row" style={{ flex: 1 }}>
                <input
                  type="checkbox"
                  checked={serial.rts ?? false}
                  onChange={(e) => patchSerial({ rts: e.target.checked })}
                />
                Поднять RTS
              </label>
              <label style={{ flex: 2 }}>
                Группа
                <input value={group} onChange={(e) => setGroup(e.target.value)} placeholder="Стойка 1" />
              </label>
            </div>

            <div className="agent-hint">
              Типовая консоль сетевого железа — 9600 8N1 без управления потоком. Платы на CH340
              и подобных часто требуют поднятых DTR/RTS.
            </div>
          </>
        ) : isTcp ? (
          <>
            <div className="row">
              <label style={{ flex: 3 }}>
                Хост
                <input
                  value={host}
                  onChange={(e) => setHost(e.target.value)}
                  placeholder="192.168.88.1 / sw1.lan"
                />
              </label>
              <label style={{ flex: 1 }}>
                Порт
                <input
                  type="number"
                  value={port || ''}
                  onChange={(e) => setPort(Number(e.target.value))}
                  placeholder={connection === 'telnet' ? '23' : '2001'}
                />
              </label>
              <label style={{ flex: 2 }}>
                Группа
                <input value={group} onChange={(e) => setGroup(e.target.value)} placeholder="Стойка 1" />
              </label>
            </div>

            {connection === 'telnet' && (
              <label>
                Клавиша Enter отправляет
                <select
                  value={telnetEol}
                  onChange={(e) => setTelnetEol(e.target.value as 'crlf' | 'cr-nul' | 'cr')}
                >
                  <option value="crlf">CR LF — по стандарту (подходит почти везде)</option>
                  <option value="cr-nul">CR NUL — если строки задваиваются</option>
                  <option value="cr">Только CR — для совсем упрямых железок</option>
                </select>
              </label>
            )}

            <div className="agent-hint">
              {connection === 'telnet'
                ? 'Логин и пароль спрашивает сама железка — здесь их указывать негде. Трафик telnet не шифруется: в чужой сети им ходить не стоит.'
                : 'Байты идут в обе стороны без обработки: ни согласования опций, ни правки перевода строки. Так подключаются к консольным серверам (Cisco и Digi слушают порт 2000+ на каждую линию) и к текстовым протоколам вручную.'}
            </div>
          </>
        ) : (
          <>
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
          </>
        )}

        {/* Пароли, ключи, jump-хост и туннели относятся только к SSH. */}
        {connection === 'ssh' && (
          <>
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
          Прокси-команда (вместо прямого подключения)
          <input
            value={proxyCommand}
            onChange={(e) => setProxyCommand(e.target.value)}
            placeholder="cloudflared access ssh --hostname %h"
            disabled={!!proxyJump}
          />
        </label>
        {proxyJump ? (
          <div className="agent-hint">
            Не используется: выбран jump-хост, он идёт первым — как в OpenSSH.
          </div>
        ) : (
          proxyCommand.trim() && (
            <div className="agent-hint">
              Подстановки: <code>%h</code> — хост, <code>%p</code> — порт, <code>%r</code> —
              пользователь. Программа запускается на этом компьютере, её ввод-вывод и служит
              каналом до сервера.
            </div>
          )
        )}

        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={sshCompression}
            onChange={(e) => setSshCompression(e.target.checked)}
          />
          Сжимать трафик (zlib) — для медленного канала
        </label>

        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={sshLegacyAlgos}
            onChange={(e) => setSshLegacyAlgos(e.target.checked)}
          />
          Разрешить устаревшие алгоритмы (старые коммутаторы и прошивки)
        </label>
        {sshLegacyAlgos && (
          <div className="agent-hint">
            Добавит <code>diffie-hellman-group1-sha1</code>, CBC-шифры, <code>3des-cbc</code> и{' '}
            <code>ssh-rsa</code> в конец списка. С современным сервером по-прежнему выберется
            сильный набор.
          </div>
        )}

        <label>
          Команда при подключении (выполнится после открытия shell)
          <input
            value={executeOnConnect}
            onChange={(e) => setExecuteOnConnect(e.target.value)}
            placeholder="cd /var/www && tmux attach || tmux"
          />
        </label>

        {authType === 'agent' && (
          <>
            <label>
              <span className="agent-label">
                Ключ из агента
                <button
                  type="button"
                  className="mini"
                  disabled={agentLoading}
                  onClick={() => void loadAgentKeys()}
                >
                  {agentLoading ? 'Читаю…' : 'Обновить'}
                </button>
              </span>
              <select value={agentKey} onChange={(e) => setAgentKey(e.target.value)}>
                <option value="">— Любой ключ из агента —</option>
                {agentKeys.map((k) => (
                  <option key={k.fingerprint} value={k.fingerprint}>
                    {k.comment || k.algo} · {k.algo} · {k.fingerprint.slice(0, 24)}…
                  </option>
                ))}
                {/* Ключ мог быть выбран раньше, а сейчас не загружен в агент —
                    не теряем настройку сервера молча. */}
                {agentKey && !agentKeys.some((k) => k.fingerprint === agentKey) && (
                  <option value={agentKey}>{agentKey.slice(0, 24)}… (сейчас не в агенте)</option>
                )}
              </select>
            </label>

            {agentError && <div className="agent-hint error">{agentError}</div>}
            {!agentError && !agentLoading && agentKeys.length === 0 && (
              <div className="agent-hint">
                В агенте нет ключей. Добавьте: <code>ssh-add путь\к\ключу</code>
              </div>
            )}
            {!agentError && agentKeys.length > 0 && !agentKey && (
              <div className="agent-hint">
                Будут перебираться все ключи подряд. Если на сервере ограничен
                <code>MaxAuthTries</code>, выберите конкретный ключ.
              </div>
            )}

            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={agentForward}
                onChange={(e) => setAgentForward(e.target.checked)}
              />
              Пробрасывать SSH-агент на сервер (agent forwarding)
            </label>
          </>
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
          </>
        )}

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
