import { useCallback, useEffect, useState } from 'react'
import { errText } from '../errText'
import { Icon } from './Icon'
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

type Screen = 'choose' | 'vnc' | 'setup'

export function DesktopPanel({ sessionId, panelTitle, onDetached, fill }: Props): JSX.Element {
  const [screen, setScreen] = useState<Screen>('choose')
  const [info, setInfo] = useState<Detected | null>(null)
  const [error, setError] = useState('')

  const detect = useCallback(async () => {
    setError('')
    try {
      setInfo((await window.api.desktop.detect(sessionId)) as unknown as Detected)
    } catch (e) {
      setError(errText(e))
    }
  }, [sessionId])

  useEffect(() => {
    void detect()
  }, [detect])

  if (screen === 'vnc') {
    return (
      <VncPanel
        sessionId={sessionId}
        panelTitle={panelTitle}
        onDetached={onDetached}
        fill={fill}
        onBack={() => setScreen('choose')}
        onSetup={() => setScreen('setup')}
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
              Идёт каналом внутри этой SSH-сессии - порт наружу открывать не нужно
            </span>
          </button>

          {/* Серая не потому, что «когда-нибудь сделаем», а потому что причина известна и
              названа: зависимости RDP конфликтуют с нашим SSH-ядром. Обещать кнопкой то,
              чего нет, хуже, чем честно показать место. */}
          <div className="desk-tile off" title="Пока недоступен">
            <Icon name="monitor" size={26} />
            <span className="desk-tile-name">RDP</span>
            <span className="desk-tile-note">
              Пока нет: библиотека RDP требует версию криптографии, несовместимую с нашим
              SSH-ядром. Ждём, пока разойдутся.
            </span>
          </div>
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
            {info.desktop === 'нет' && (
              <div className="hint">
                Графической среды на сервере не видно - показывать рабочий стол будет нечего,
                даже если VNC запустится.
              </div>
            )}
            <button className="mini" onClick={() => setScreen('setup')}>
              <Icon name="settings" size={13} /> Настроить VNC
            </button>
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
            Установить отсюда не выйдет: {info.packageManager ? '' : 'не нашли, чем ставить пакеты'}
            {!info.packageManager && info.sudo === 'нет' ? ', и ' : ''}
            {info.sudo === 'нет' ? 'у этого пользователя нет прав sudo' : ''}. Поставьте VNC на
            сервере обычным способом и вернитесь.
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
            <p className="hint">
              Пароль уходит на стандартный ввод команды, а не в её строку: строка команды
              целиком видна в списке процессов сервера. Нигде не сохраняется.
            </p>
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
            Восемь символов - предел самого протокола: VNC шифрует пароль ключом такой длины
            и молча обрезает лишнее. Поэтому же порт и не открывают наружу.
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
