import { useEffect, useState } from 'react'
import type { KnownHostEntry } from '../../shared/types'
import { useSettings } from '../SettingsContext'
import { THEME_NAMES } from '../themes'
import { ACTIONS, resolveBindings, comboFromEvent, formatCombo, type ActionId } from '../keybindings'
import { checkForUpdates } from '../updater'
import { getVersion } from '@tauri-apps/api/app'
import { errText } from '../errText'
import { appPlatform } from '../platform'

const FONTS = [
  'Cascadia Code, Consolas, "Courier New", monospace',
  'Consolas, monospace',
  'JetBrains Mono, monospace',
  '"Fira Code", monospace',
  '"Courier New", monospace'
]

type Action = null | 'enable' | 'disable' | 'export' | 'import'

export function SettingsModal({ onClose }: { onClose: () => void }): JSX.Element {
  const { settings, update } = useSettings()
  const [recording, setRecording] = useState<ActionId | null>(null)
  const [knownHosts, setKnownHosts] = useState<KnownHostEntry[]>([])
  const [hostsMsg, setHostsMsg] = useState('')
  const [platform, setPlatform] = useState<'windows' | 'linux' | 'other'>('windows')

  useEffect(() => {
    void appPlatform().then(setPlatform)
  }, [])

  const reloadKnownHosts = async (): Promise<void> => {
    setKnownHosts(await window.api.knownHosts.list())
  }
  useEffect(() => {
    void reloadKnownHosts()
  }, [])
  const [masterEnabled, setMasterEnabled] = useState(false)
  const [action, setAction] = useState<Action>(null)
  const [password, setPassword] = useState('')
  const [msg, setMsg] = useState<{ text: string; ok: boolean } | null>(null)
  const [busy, setBusy] = useState(false)
  const [appVersion, setAppVersion] = useState('')
  const [paths, setPaths] = useState<{ config: string; logs: string } | null>(null)
  const [checkingUpdate, setCheckingUpdate] = useState(false)

  useEffect(() => {
    window.api.vault.status().then((s) => setMasterEnabled(s.enabled))
    getVersion().then(setAppVersion).catch(() => {})
    window.api.app.paths().then(setPaths).catch(() => {})
  }, [])

  const onCheckUpdate = async (): Promise<void> => {
    setCheckingUpdate(true)
    try {
      await checkForUpdates(false)
    } finally {
      setCheckingUpdate(false)
    }
  }

  const startAction = (a: Action): void => {
    setAction(a)
    setPassword('')
    setMsg(null)
  }

  const runAction = async (): Promise<void> => {
    if (!password) return
    setBusy(true)
    setMsg(null)
    try {
      if (action === 'enable') {
        const r = await window.api.vault.enable(password)
        if (r.ok) {
          setMasterEnabled(true)
          setMsg({ text: 'Мастер-пароль включён', ok: true })
        } else setMsg({ text: r.error || 'Ошибка', ok: false })
      } else if (action === 'disable') {
        const r = await window.api.vault.disable(password)
        if (r.ok) {
          setMasterEnabled(false)
          setMsg({ text: 'Мастер-пароль отключён', ok: true })
        } else setMsg({ text: r.error || 'Ошибка', ok: false })
      } else if (action === 'export') {
        const r = await window.api.backup.export(password)
        setMsg(r.saved ? { text: `Бэкап сохранён: ${r.path}`, ok: true } : { text: 'Отменено', ok: false })
      } else if (action === 'import') {
        const r = await window.api.backup.import(password)
        if (r.imported) {
          // Про переставленные пути к ключам говорим прямо: иначе непонятно, почему
          // в профиле теперь другой путь, чем был на прежней машине.
          const remapped = r.keysRemapped
            ? `; путей к ключам поправлено под эту систему: ${r.keysRemapped}`
            : ''
          setMsg({
            text: `Импортировано серверов: ${r.servers}, сниппетов: ${r.snippets}${remapped}`,
            ok: true
          })
        } else setMsg({ text: 'Отменено', ok: false })
      }
      setAction(null)
      setPassword('')
    } catch (e) {
      setMsg({ text: errText(e), ok: false })
    } finally {
      setBusy(false)
    }
  }

  // --- Горячие клавиши ---
  const bindings = resolveBindings(settings)
  const overrides = settings.keybindings ?? {}
  const setBinding = (id: ActionId, combo: string): void =>
    update({ keybindings: { ...overrides, [id]: combo } })
  const resetBinding = (id: ActionId): void => {
    const next = { ...overrides }
    delete next[id]
    update({ keybindings: next })
  }
  const onCaptureKey = (id: ActionId, e: React.KeyboardEvent): void => {
    e.preventDefault()
    if (e.key === 'Escape') return setRecording(null)
    if (e.key === 'Backspace' || e.key === 'Delete') {
      resetBinding(id)
      return setRecording(null)
    }
    const combo = comboFromEvent(e)
    if (!combo) return // нажат только модификатор — ждём основную клавишу
    setBinding(id, combo)
    setRecording(null)
  }

  const actionTitle: Record<Exclude<Action, null>, string> = {
    enable: 'Задайте мастер-пароль',
    disable: 'Введите текущий мастер-пароль',
    export: 'Пароль для шифрования бэкапа',
    import: 'Пароль от файла бэкапа'
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Настройки</h2>

        <label>
          Цветовая схема
          <select value={settings.theme} onChange={(e) => update({ theme: e.target.value })}>
            {THEME_NAMES.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        </label>

        <label>
          Шрифт
          <select value={settings.fontFamily} onChange={(e) => update({ fontFamily: e.target.value })}>
            {FONTS.map((f) => (
              <option key={f} value={f}>
                {f.split(',')[0].replace(/"/g, '')}
              </option>
            ))}
          </select>
        </label>

        <label>
          Размер шрифта: {settings.fontSize}px
          <input
            type="range"
            min={8}
            max={32}
            value={settings.fontSize}
            onChange={(e) => update({ fontSize: Number(e.target.value) })}
          />
        </label>

        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={settings.openLocalOnStart}
            onChange={(e) => update({ openLocalOnStart: e.target.checked })}
          />
          Открывать локальный терминал при запуске
        </label>

        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={settings.autoReconnect}
            onChange={(e) => update({ autoReconnect: e.target.checked })}
          />
          Авто-переподключение SSH при обрыве (до 5 попыток)
        </label>

        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={!!settings.restoreTabsOnStart}
            onChange={(e) => update({ restoreTabsOnStart: e.target.checked })}
          />
          Восстанавливать вкладки при запуске
        </label>

        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={!!settings.restoreAuxOnStart}
            onChange={(e) => update({ restoreAuxOnStart: e.target.checked })}
          />
          Сохранять расположение доп. панелей после перезапуска
        </label>
        <div className="settings-row-desc" style={{ marginTop: -8, marginBottom: 10 }}>
          SFTP в главном окне и откреплённые окна (SFTP, логи) откроются как были: примагниченные — рядом, остальные — на прежних местах.
        </div>

        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={settings.density === 'compact'}
            onChange={(e) => update({ density: e.target.checked ? 'compact' : 'comfortable' })}
          />
          Компактный режим (плотнее интерфейс)
        </label>

        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={!!settings.auxInTaskbar}
            onChange={(e) => update({ auxInTaskbar: e.target.checked })}
          />
          Отдельные кнопки на панели задач для откреплённых окон
        </label>
        <div className="settings-row-desc" style={{ marginTop: -8, marginBottom: 10 }}>
          По умолчанию одна кнопка на панели задач (откреплённые окна без своей иконки). Клик по Serein на панели или кнопка внизу главного окна разворачивает свёрнутые. Галочка — отдельная кнопка у каждого окна. Переоткрой уже откреплённые окна после смены.
        </div>

        <label>
          Параллельных SFTP-передач: {Math.min(8, Math.max(1, settings.sftpConcurrency ?? 4))}
          <input
            type="range"
            min={1}
            max={8}
            value={Math.min(8, Math.max(1, settings.sftpConcurrency ?? 4))}
            onChange={(e) => update({ sftpConcurrency: Number(e.target.value) })}
          />
        </label>
        <div className="settings-row-desc" style={{ marginTop: -8, marginBottom: 10 }}>
          Одновременно копируемых файлов. Больше — быстрее папки, тяжелее канал SSH.
        </div>

        <div className="settings-section">
          <div className="settings-section-title">SFTP-проводник</div>
          <div className="settings-row-desc" style={{ marginBottom: 10 }}>
            Как в проводнике Windows: колонки, сортировка по заголовку, ширина тянется за край. Имя нельзя выключить.
          </div>
          {(['name', 'ext', 'mode', 'size', 'mtime'] as const).map((id) => {
            const label =
              id === 'name'
                ? 'Имя'
                : id === 'ext'
                  ? 'Тип (расширение)'
                  : id === 'mode'
                    ? 'Права'
                    : id === 'size'
                      ? 'Размер'
                      : 'Дата изменения'
            const on = id === 'name' ? true : settings.sftpColOn?.[id] !== false
            return (
              <label key={id} className="checkbox-row">
                <input
                  type="checkbox"
                  checked={on}
                  disabled={id === 'name'}
                  onChange={(e) =>
                    update({
                      sftpColOn: {
                        name: true,
                        ext: settings.sftpColOn?.ext !== false,
                        mode: settings.sftpColOn?.mode !== false,
                        size: settings.sftpColOn?.size !== false,
                        mtime: settings.sftpColOn?.mtime !== false,
                        [id]: e.target.checked,
                      },
                    })
                  }
                />
                {label}
              </label>
            )
          })}
        </div>

        <label>
          {platform === 'linux' ? 'Shell локального терминала' : 'Shell локального терминала (Windows)'}
          <select value={settings.localShell ?? 'auto'} onChange={(e) => update({ localShell: e.target.value })}>
            {platform === 'linux' ? (
              <>
                <option value="auto">Авто ($SHELL)</option>
                <option value="bash">/bin/bash</option>
                <option value="zsh">/bin/zsh</option>
                <option value="fish">/usr/bin/fish</option>
              </>
            ) : (
              <>
                <option value="auto">Авто (PowerShell → cmd)</option>
                <option value="pwsh">PowerShell 7 (pwsh)</option>
                <option value="powershell">Windows PowerShell</option>
                <option value="cmd">cmd.exe</option>
                <option value="wsl">WSL</option>
              </>
            )}
          </select>
        </label>

        {/* ---- Горячие клавиши ---- */}
        <div className="settings-section">
          <div className="settings-section-title">Горячие клавиши</div>
          {ACTIONS.map((a) => {
            const combo = bindings[a.id]
            const overridden = overrides[a.id] != null
            const dup = Object.entries(bindings).some(([oid, c]) => oid !== a.id && c === combo)
            return (
              <div key={a.id} className="kb-row">
                <span className="kb-label">{a.label}</span>
                {recording === a.id ? (
                  <input
                    className="kb-capture"
                    data-keycapture
                    autoFocus
                    readOnly
                    value="Нажмите клавиши…"
                    onKeyDown={(e) => onCaptureKey(a.id, e)}
                    onBlur={() => setRecording(null)}
                  />
                ) : (
                  <button
                    className={'kb-combo' + (dup ? ' dup' : '')}
                    title={dup ? 'Конфликт: комбинация занята другим действием' : 'Нажмите, чтобы изменить'}
                    onClick={() => setRecording(a.id)}
                  >
                    {formatCombo(combo)}
                    {dup ? ' ⚠' : ''}
                  </button>
                )}
                {overridden && recording !== a.id && (
                  <button className="mini" title="Сбросить к умолчанию" onClick={() => resetBinding(a.id)}>
                    ↺
                  </button>
                )}
              </div>
            )
          })}
          <div className="settings-row-desc" style={{ marginTop: 4 }}>
            При записи: Esc — отмена, Backspace — сброс к умолчанию.
          </div>
        </div>

        {/* ---- Безопасность и хранение ---- */}
        <div className="settings-section">
          <div className="settings-section-title">Безопасность и хранение</div>

          <div className="settings-row">
            <div>
              <div className="settings-row-name">Мастер-пароль</div>
              <div className="settings-row-desc">
                {masterEnabled ? 'Включён — спрашивается при запуске' : 'Доп. шифрование секретов поверх системного'}
              </div>
            </div>
            {masterEnabled ? (
              <button className="secondary" onClick={() => startAction('disable')}>Отключить</button>
            ) : (
              <button className="secondary" onClick={() => startAction('enable')}>Включить</button>
            )}
          </div>

          <div className="settings-row">
            <div>
              <div className="settings-row-name">Резервная копия</div>
              {/* Про ключи говорим здесь, а не после неудачного восстановления: файлы ключей
                  в бэкап не кладём намеренно — он и так несёт пароли, и приватные ключи в том
                  же файле сильно повышают цену его утечки. */}
              <div className="settings-row-desc">
                Зашифрованный бэкап серверов, настроек и сниппетов.
                <br />
                Файлы SSH-ключей в него не входят — перенесите их отдельно, пути к ним при
                восстановлении подставятся автоматически.
              </div>
            </div>
            <div style={{ display: 'flex', gap: 6 }}>
              <button className="secondary" onClick={() => startAction('export')}>Экспорт</button>
              <button className="secondary" onClick={() => startAction('import')}>Импорт</button>
            </div>
          </div>

          {action && (
            <div className="settings-action-form">
              <label>
                {actionTitle[action]}
                <input
                  type="password"
                  autoFocus
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') void runAction()
                    if (e.key === 'Escape') setAction(null)
                  }}
                />
              </label>
              <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
                <button className="secondary" onClick={() => setAction(null)}>Отмена</button>
                <button className="primary" disabled={busy} onClick={() => void runAction()}>
                  {busy ? '…' : 'OK'}
                </button>
              </div>
            </div>
          )}

          {msg && <div className={'settings-msg' + (msg.ok ? ' ok' : ' err')}>{msg.text}</div>}
        </div>

        {/* ---- Обновления ---- */}
        <div className="settings-section">
          <div className="settings-section-title">Ключи серверов (known hosts)</div>

          <div className="settings-row">
            <div>
              <div className="settings-row-name">Запомненные отпечатки</div>
              <div className="settings-row-desc">
                {knownHosts.length
                  ? `Хостов: ${knownHosts.length}. Забыть — и при следующем подключении ключ спросят заново.`
                  : 'Пока пусто: отпечаток запоминается после подтверждения при подключении.'}
                {' '}Можно перенести уже доверенные из <code>~/.ssh/known_hosts</code>.
              </div>
            </div>
            <button
              className="mini"
              onClick={() => {
                void window.api.knownHosts
                  .importOpenssh()
                  .then((r) => {
                    setHostsMsg(
                      r.imported ? `Импортировано записей: ${r.imported}` : 'Новых записей не нашлось'
                    )
                    return reloadKnownHosts()
                  })
                  .catch((e: Error) => setHostsMsg(e.message))
              }}
            >
              Импорт
            </button>
          </div>

          {hostsMsg && <div className="settings-row-desc">{hostsMsg}</div>}

          {knownHosts.length > 0 && (
            <div className="known-hosts-list">
              {knownHosts.map((h) => (
                <div key={h.host} className="known-host-row">
                  <span className="known-host-name">{h.host}</span>
                  <span className="known-host-fp" title={h.fingerprint}>
                    {h.fingerprint}
                  </span>
                  <button
                    className="mini danger"
                    title="Забыть этот хост"
                    onClick={() => {
                      void window.api.knownHosts.forget(h.host).then(() => {
                        setHostsMsg(`Забыт: ${h.host}`)
                        return reloadKnownHosts()
                      })
                    }}
                  >
                    ✕
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="settings-section">
          <div className="settings-section-title">Обновления</div>
          <div className="settings-row">
            <div>
              <div className="settings-row-name">Serein{appVersion ? ' ' + appVersion : ''}</div>
              <div className="settings-row-desc">Проверить наличие новой версии и обновиться</div>
            </div>
            <button className="secondary" disabled={checkingUpdate} onClick={() => void onCheckUpdate()}>
              {checkingUpdate ? '…' : 'Проверить обновления'}
            </button>
          </div>

          {/* Путь к профилю — не украшение: если приложение запустили из окружения
              с другим HOME (на Linux так бывает при старте из меню), оно молча
              откроет пустой профиль, и по этой строке это видно сразу. */}
          {paths && (
            <div className="settings-row">
              <div>
                <div className="settings-row-name">Профиль и логи</div>
                <div className="settings-row-desc settings-path">{paths.config}</div>
              </div>
              <button
                className="secondary"
                onClick={() => void window.api.clipboard.write(paths.config)}
              >
                Скопировать путь
              </button>
            </div>
          )}
        </div>

        <div className="modal-actions">
          <button className="primary" onClick={onClose}>
            Готово
          </button>
        </div>
      </div>
    </div>
  )
}
