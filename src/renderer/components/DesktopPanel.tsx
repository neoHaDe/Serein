import { useCallback, useEffect, useState } from 'react'
import { errText } from '../errText'
import { Icon } from './Icon'
import { RdpPanel } from './RdpPanel'
import { VncPanel } from './VncPanel'

/**
 * Выбор рабочего стола: чем подключаться и всё ли для этого готово.
 *
 * Раньше нажатие на Desktop сразу лезло подключаться по VNC. Это неудобно по двум
 * причинам. Во-первых, способов будет два - RDP ждёт своей очереди, и место под него
 * должно быть видно заранее, а не появиться однажды сюрпризом. Во-вторых, если на сервере
 * VNC нет вовсе, единственным ответом была невнятная ошибка соединения; теперь панель
 * сначала смотрит, что там есть, и говорит словами, чего не хватает.
 */
interface Props {
  sessionId: string
  panelTitle?: string
  onDetached?: () => void
  fill?: boolean
}

interface Detected {
  installed: { name: string; path: string }[]
  listening: string[]
  desktop?: string
  packageManager?: string
  sudo?: string
  summary: string
  canInstall: boolean
}

interface RdpDetected {
  installed: { name: string; path: string }[]
  listening: string[]
  service?: string
  desktop?: string
  packageManager?: string
  sudo?: string
  summary: string
  canInstall: boolean
  canStart: boolean
}

type Screen = 'choose' | 'vnc' | 'rdp' | 'setup' | 'rdp-setup'

export function DesktopPanel({ sessionId, panelTitle, onDetached, fill }: Props): JSX.Element {
  const [screen, setScreen] = useState<Screen>('choose')
  const [info, setInfo] = useState<Detected | null>(null)
  const [rdp, setRdp] = useState<RdpDetected | null>(null)
  const [existing, setExisting] = useState<{ kind: 'vnc' | 'rdp'; id: string } | null>(null)
  const [error, setError] = useState('')

  // Два вопроса серверу вместо одного, зато оба с внятным ответом. Идут разом: ждать
  // второй ответ после первого незачем, они друг от друга не зависят.
  const detect = useCallback(async () => {
    setError('')
    const [vnc, rdpInfo] = await Promise.allSettled([
      window.api.desktop.detect(sessionId),
      window.api.desktop.rdpDetect(sessionId)
    ])
    if (vnc.status === 'fulfilled') setInfo(vnc.value as unknown as Detected)
    if (rdpInfo.status === 'fulfilled') setRdp(rdpInfo.value as unknown as RdpDetected)
    // Жалуемся один раз и на первом же отказе: две одинаковые ошибки подряд - шум.
    const failed = [vnc, rdpInfo].find((r) => r.status === 'rejected')
    if (failed && failed.status === 'rejected') setError(errText(failed.reason))
  }, [sessionId])

  useEffect(() => {
    void detect()
  }, [detect])

  // Уже открытый рабочий стол этой сессии. Есть - открываем сразу его, не спрашивая
  // ничего заново: сеанс живёт в приложении, а не в окне, и откреплённое окно должно
  // продолжить картинку, а не начинать с пароля.
  useEffect(() => {
    let gone = false
    void (async () => {
      const a = await window.api.desktop.active(sessionId).catch(() => null)
      if (!gone && a) {
        setExisting(a)
        setScreen(a.kind)
      }
    })()
    return () => {
      gone = true
    }
  }, [sessionId])

  if (screen === 'vnc') {
    return (
      <VncPanel
        sessionId={sessionId}
        panelTitle={panelTitle}
        onDetached={onDetached}
        fill={fill}
        existingId={existing?.kind === 'vnc' ? existing.id : undefined}
        onBack={() => {
          setExisting(null)
          setScreen('choose')
        }}
        onSetup={() => setScreen('setup')}
      />
    )
  }

  if (screen === 'rdp') {
    return (
      <RdpPanel
        sessionId={sessionId}
        panelTitle={panelTitle}
        onDetached={onDetached}
        fill={fill}
        existingId={existing?.kind === 'rdp' ? existing.id : undefined}
        onBack={() => {
          setExisting(null)
          setScreen('choose')
        }}
      />
    )
  }

  if (screen === 'rdp-setup') {
    return (
      <RdpSetup
        sessionId={sessionId}
        info={rdp}
        onRefresh={detect}
        onClose={() => setScreen('choose')}
      />
    )
  }

  if (screen === 'setup') {
    return (
      <VncSetup
        sessionId={sessionId}
        info={info}
        onRefresh={detect}
        onClose={() => setScreen('choose')}
      />
    )
  }

  return (
    <div className={'ws-panel' + (fill ? ' fill' : '')}>
      <div className="ws-head">
        <span className="ws-head-title">
          <Icon name="desktop" size={15} /> Рабочий стол
        </span>
      </div>

      <div className="desk-choose">
        <div className="desk-tiles">
          <button className="desk-tile" onClick={() => setScreen('vnc')}>
            <Icon name="desktop" size={26} />
            <span className="desk-tile-name">VNC</span>
            <span className="desk-tile-note">
              Вход через канал SSH. Проброс порта не требуется
            </span>
          </button>

          <button className="desk-tile" onClick={() => setScreen('rdp')}>
            <Icon name="monitor" size={26} />
            <span className="desk-tile-name">RDP</span>
            <span className="desk-tile-note">
              Подключение внутри SSH-канала. Вход по учётной записи сервера
            </span>
          </button>
        </div>

        {error && <div className="sftp-error">{error}</div>}

        {info && (
          <div className="desk-state">
            <div className="desk-state-line">
              <Icon name={info.listening.length ? 'check' : 'bolt'} size={14} />
              <span>{info.summary}</span>
            </div>
            {info.listening.length > 0 && (
              <div className="hint">Слушает: {info.listening.join(', ')}</div>
            )}
            <div className="desk-state-line">
              <Icon name={rdp?.listening.length ? 'check' : 'bolt'} size={14} />
              <span>{rdp?.summary ?? 'Смотрю, что там с RDP…'}</span>
            </div>
            {rdp && rdp.listening.length > 0 && (
              <div className="hint">Слушает: {rdp.listening.join(', ')}</div>
            )}
            {info.desktop === 'нет' && (
              <div className="hint">Графической среды на сервере нет.</div>
            )}
            <div className="desk-state-buttons">
              <button className="mini" onClick={() => setScreen('setup')}>
                <Icon name="settings" size={13} /> Настроить VNC
              </button>
              <button className="mini" onClick={() => setScreen('rdp-setup')}>
                <Icon name="settings" size={13} /> Настроить RDP
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

/**
 * Настройка VNC на сервере: поставить, если нет, и задать пароль.
 *
 * Всё, что меняет систему, спрашивает пароль sudo - он уходит на стандартный ввод и
 * нигде не сохраняется. Пароль в поле живёт ровно до конца операции.
 */
function VncSetup({
  sessionId,
  info,
  onRefresh,
  onClose
}: {
  sessionId: string
  info: Detected | null
  onRefresh: () => Promise<void>
  onClose: () => void
}): JSX.Element {
  const [sudo, setSudo] = useState('')
  const [vncPass, setVncPass] = useState('')
  const [busy, setBusy] = useState('')
  const [msg, setMsg] = useState('')
  const [error, setError] = useState('')

  const install = async (): Promise<void> => {
    if (!info?.packageManager) return
    setBusy('install')
    setError('')
    setMsg('')
    try {
      const r = await window.api.desktop.install(sessionId, info.packageManager, sudo)
      if (r.ok) {
        setMsg('Поставили. Осталось задать пароль и запустить.')
        // Пароль sudo больше не нужен - держать его в поле незачем.
        setSudo('')
        await onRefresh()
      } else setError(String(r.error ?? 'установка не удалась'))
    } catch (e) {
      setError(errText(e))
    } finally {
      setBusy('')
    }
  }

  const setPassword = async (): Promise<void> => {
    setBusy('pass')
    setError('')
    setMsg('')
    try {
      const r = await window.api.desktop.setPassword(sessionId, vncPass)
      if (r.ok) {
        setMsg('Пароль рабочего стола сохранён.')
        setVncPass('')
      } else setError(String(r.error ?? 'не удалось сохранить пароль'))
    } catch (e) {
      setError(errText(e))
    } finally {
      setBusy('')
    }
  }

  return (
    <div className="ws-panel fill">
      <div className="ws-head">
        <span className="ws-head-title">
          <Icon name="settings" size={15} /> Настройка VNC
        </span>
        <button className="mini" onClick={onClose}>
          <Icon name="back" size={14} /> Назад
        </button>
      </div>

      <div className="desk-setup">
        <div className="desk-state">
          <div className="desk-state-line">
            <Icon name="server" size={14} />
            <span>{info?.summary ?? 'Смотрю, что на сервере…'}</span>
          </div>
          {info && (
            <div className="hint">
              {info.installed.length > 0
                ? `Найдено: ${info.installed.map((b) => b.name).join(', ')}`
                : 'Программ VNC не найдено'}
              {info.packageManager ? ` · пакеты ставит ${info.packageManager}` : ''}
              {info.sudo ? ` · sudo: ${info.sudo}` : ''}
            </div>
          )}
        </div>

        {info && !info.canInstall && info.installed.length === 0 && (
          <p className="hint">
            Установка отсюда недоступна:{' '}
            {info.packageManager ? '' : 'не найден менеджер пакетов'}
            {!info.packageManager && info.sudo === 'нет' ? ', ' : ''}
            {info.sudo === 'нет' ? 'нет прав sudo' : ''}.
          </p>
        )}

        {info?.canInstall && info.installed.length === 0 && (
          <div className="desk-block">
            <span className="desk-block-title">Установить сервер VNC</span>
            <label>
              Пароль sudo
              <input
                type="password"
                value={sudo}
                onChange={(e) => setSudo(e.target.value)}
                placeholder={info.sudo === 'без пароля' ? 'не требуется' : ''}
              />
            </label>
            <p className="hint">Пароль не сохраняется.</p>
            <button
              className="primary"
              disabled={busy !== '' || (info.sudo !== 'без пароля' && !sudo)}
              onClick={() => void install()}
            >
              {busy === 'install' ? 'Ставлю…' : `Поставить ${info.packageManager === 'apt-get' ? 'x11vnc' : 'tigervnc'}`}
            </button>
          </div>
        )}

        <div className="desk-block">
          <span className="desk-block-title">Пароль рабочего стола</span>
          <label>
            Не длиннее восьми символов
            <input type="password" value={vncPass} onChange={(e) => setVncPass(e.target.value)} />
          </label>
          <p className="hint">
            Длиннее восьми символов протокол не хранит. После нескольких неудачных попыток
            входа сервер закрывает доступ до перезапуска службы.
          </p>
          <button className="primary" disabled={busy !== '' || !vncPass} onClick={() => void setPassword()}>
            {busy === 'pass' ? 'Сохраняю…' : 'Сохранить пароль'}
          </button>
        </div>

        {msg && <div className="desk-ok">{msg}</div>}
        {error && <div className="sftp-error">{error}</div>}
      </div>
    </div>
  )
}

/**
 * Настройка RDP на сервере: поставить, если нет, и запустить.
 *
 * Короче соседнего экрана для VNC, и по делу: своего пароля у RDP нет вовсе - вход идёт
 * по обычной учётной записи сервера. Задавать здесь нечего, и это его главное удобство.
 */
function RdpSetup({
  sessionId,
  info,
  onRefresh,
  onClose
}: {
  sessionId: string
  info: RdpDetected | null
  onRefresh: () => Promise<void>
  onClose: () => void
}): JSX.Element {
  const [sudo, setSudo] = useState('')
  const [busy, setBusy] = useState('')
  const [msg, setMsg] = useState('')
  const [error, setError] = useState('')

  const run = async (
    what: 'install' | 'start',
    action: () => Promise<{ ok: boolean; error?: string }>,
    done: string
  ): Promise<void> => {
    setBusy(what)
    setError('')
    setMsg('')
    try {
      const r = await action()
      if (r.ok) {
        setMsg(done)
        // Пароль sudo больше не нужен - держать его в поле незачем.
        setSudo('')
        await onRefresh()
      } else setError(String(r.error ?? 'не получилось'))
    } catch (e) {
      setError(errText(e))
    } finally {
      setBusy('')
    }
  }

  const needSudo = info?.sudo !== 'без пароля' && !sudo

  return (
    <div className="ws-panel fill">
      <div className="ws-head">
        <span className="ws-head-title">
          <Icon name="settings" size={15} /> Настройка RDP
        </span>
        <button className="mini" onClick={onClose}>
          <Icon name="back" size={14} /> Назад
        </button>
      </div>

      <div className="desk-setup">
        <div className="desk-state">
          <div className="desk-state-line">
            <Icon name="server" size={14} />
            <span>{info?.summary ?? 'Смотрю, что на сервере…'}</span>
          </div>
          {info && (
            <div className="hint">
              {info.installed.length > 0
                ? `Найдено: ${info.installed.map((b) => b.name).join(', ')}`
                : 'xrdp не найден'}
              {info.service ? ` · служба: ${info.service}` : ''}
              {info.packageManager ? ` · пакеты ставит ${info.packageManager}` : ''}
              {info.sudo ? ` · sudo: ${info.sudo}` : ''}
            </div>
          )}
        </div>

        {info && !info.canInstall && info.installed.length === 0 && (
          <p className="hint">
            Установка отсюда недоступна:{' '}
            {info.packageManager === 'pacman'
              ? 'в основных хранилищах Arch пакета xrdp нет, только в AUR.'
              : 'не найден менеджер пакетов или нет прав sudo.'}
          </p>
        )}

        {(info?.canInstall || info?.canStart) && (
          <div className="desk-block">
            <span className="desk-block-title">Пароль sudo</span>
            <label>
              Для установки и запуска службы
              <input
                type="password"
                value={sudo}
                onChange={(e) => setSudo(e.target.value)}
                placeholder={info.sudo === 'без пароля' ? 'не требуется' : ''}
              />
            </label>
            <p className="hint">Пароль не сохраняется.</p>
          </div>
        )}

        {info?.canInstall && info.installed.length === 0 && (
          <div className="desk-block">
            <span className="desk-block-title">Установить xrdp</span>
            <p className="hint">Ставится вместе с xorgxrdp.</p>
            <button
              className="primary"
              disabled={busy !== '' || needSudo}
              onClick={() =>
                void run(
                  'install',
                  () => window.api.desktop.rdpInstall(sessionId, info.packageManager ?? '', sudo),
                  'Поставили. Осталось запустить службу.'
                )
              }
            >
              {busy === 'install' ? 'Ставлю…' : 'Поставить xrdp'}
            </button>
          </div>
        )}

        {info?.canStart && (
          <div className="desk-block">
            <span className="desk-block-title">Запустить рабочий стол</span>
            <p className="hint">Служба включится и будет подниматься при старте сервера.</p>
            <button
              className="primary"
              disabled={busy !== '' || needSudo}
              onClick={() =>
                void run(
                  'start',
                  () => window.api.desktop.rdpStart(sessionId, sudo),
                  'Служба запущена - можно подключаться.'
                )
              }
            >
              {busy === 'start' ? 'Запускаю…' : 'Запустить и включить'}
            </button>
          </div>
        )}

        <p className="hint">Вход по учётной записи сервера, отдельного пароля нет.</p>

        {msg && <div className="desk-ok">{msg}</div>}
        {error && <div className="sftp-error">{error}</div>}
      </div>
    </div>
  )
}
