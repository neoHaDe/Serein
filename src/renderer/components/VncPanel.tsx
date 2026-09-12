import { useCallback, useEffect, useRef, useState } from 'react'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { useVisible } from '../hooks/useVisible'
import { openDetachedWorkspace } from './workspaceWindow'
import { errText } from '../errText'
import { parseFrame } from '../vncFrames'
import { buttonMask, keysymFor, wheelMask } from '../vncKeys'
import { isModifier } from '../rdpKeys'
import { useFullscreen } from '../useFullscreen'

/**
 * Рабочий стол VNC внутри вкладки сервера.
 *
 * Идёт через уже открытую SSH-сессию, а не отдельным соединением: VNC на сервере обычно
 * слушает `127.0.0.1`, и это правильно - его собственная защита слабая (пароль до восьми
 * символов на DES), выставлять её в сеть незачем. Разбор кадров и клавиш живёт в
 * `vncFrames.ts` и `vncKeys.ts` - там же тесты.
 */

interface Props {
  sessionId: string
  panelTitle?: string
  onDetached?: () => void
  /** В откреплённом окне панель занимает его целиком. */
  fill?: boolean
  /**
   * Номер уже открытого сеанса. Если он есть - подключаться заново не нужно: панель
   * забирает кадры живого стола себе. Так открепление окна не рвёт сеанс.
   */
  existingId?: string
  /** Вернуться к выбору способа подключения. В откреплённом окне выбора нет. */
  onBack?: () => void
  /** Открыть настройку VNC на сервере. */
  onSetup?: () => void
}

/** Экран рисуется в свой холст и оттуда копируется на видимый - так проще с масштабом. */
interface Screen {
  canvas: HTMLCanvasElement
  ctx: CanvasRenderingContext2D
}

export function VncPanel({
  sessionId,
  panelTitle,
  onDetached,
  fill,
  existingId,
  onBack,
  onSetup
}: Props): JSX.Element {
  const [rootRef, visible] = useVisible<HTMLDivElement>()
  const viewRef = useRef<HTMLCanvasElement | null>(null)
  const screenRef = useRef<Screen | null>(null)
  const idRef = useRef<string | null>(null)
  const sizeRef = useRef<{ w: number; h: number }>({ w: 0, h: 0 })
  // Что мы считаем нажатым на той стороне: при потере фокуса это надо отпустить.
  const heldRef = useRef<Set<number>>(new Set())

  const [status, setStatus] = useState<'connecting' | 'live' | 'closed'>('connecting')
  const [error, setError] = useState('')
  const [password, setPassword] = useState('')
  const [needPassword, setNeedPassword] = useState(false)
  // Сервер временно закрыл доступ после неудачных попыток. Отдельно от «нужен пароль»,
  // потому что совет противоположный: вводить что-либо сейчас бесполезно.
  const [blocked, setBlocked] = useState(false)
  const [scaled, setScaled] = useState(true)
  const { full, toggle: toggleFull } = useFullscreen(rootRef)

  /** Переносит внутренний холст на видимый, вписывая или показывая один к одному. */
  const present = useCallback(() => {
    const view = viewRef.current
    const screen = screenRef.current
    if (!view || !screen) return
    const { w, h } = sizeRef.current
    if (!w || !h) return
    const box = view.parentElement
    if (!box) return

    const dpr = window.devicePixelRatio || 1
    const cw = box.clientWidth
    const ch = box.clientHeight
    view.width = Math.max(1, Math.floor(cw * dpr))
    view.height = Math.max(1, Math.floor(ch * dpr))
    view.style.width = `${cw}px`
    view.style.height = `${ch}px`

    const ctx = view.getContext('2d')
    if (!ctx) return
    ctx.imageSmoothingEnabled = false
    ctx.clearRect(0, 0, view.width, view.height)

    const k = scaled ? Math.min((cw * dpr) / w, (ch * dpr) / h) : dpr
    const dw = w * k
    const dh = h * k
    ctx.drawImage(screen.canvas, (view.width - dw) / 2, (view.height - dh) / 2, dw, dh)
  }, [scaled])

  /** Экранная точка → координата удалённого экрана. Обратное к present. */
  const toRemote = useCallback(
    (e: { clientX: number; clientY: number }): { x: number; y: number } | null => {
      const view = viewRef.current
      const { w, h } = sizeRef.current
      if (!view || !w || !h) return null
      const box = view.getBoundingClientRect()
      const dpr = window.devicePixelRatio || 1
      const k = scaled ? Math.min((box.width * dpr) / w, (box.height * dpr) / h) : dpr
      const dw = (w * k) / dpr
      const dh = (h * k) / dpr
      const x = ((e.clientX - box.left - (box.width - dw) / 2) / dw) * w
      const y = ((e.clientY - box.top - (box.height - dh) / 2) / dh) * h
      if (x < 0 || y < 0 || x >= w || y >= h) return null
      return { x: Math.floor(x), y: Math.floor(y) }
    },
    [scaled]
  )

  const draw = useCallback(
    (buf: ArrayBuffer) => {
      const f = parseFrame(buf)
      if (!f) return
      const screen = screenRef.current

      switch (f.kind) {
        case 'resize': {
          sizeRef.current = { w: f.w, h: f.h }
          const canvas = document.createElement('canvas')
          canvas.width = f.w
          canvas.height = f.h
          const ctx = canvas.getContext('2d')
          if (!ctx) return
          // Прежнее содержимое переносим: сервер после смены размера присылает только
          // изменившиеся области, а не весь экран заново.
          if (screen) ctx.drawImage(screen.canvas, 0, 0)
          screenRef.current = { canvas, ctx }
          setStatus('live')
          present()
          // Фокус сразу на холст: иначе клавиатура молчит до первого клика по экрану, и
          // это читается как «ввод не работает», а не «нажмите сюда».
          viewRef.current?.focus()
          return
        }
        case 'raw': {
          if (!screen) return
          const img = new ImageData(f.pixels, f.rect.w, f.rect.h)
          screen.ctx.putImageData(img, f.rect.x, f.rect.y)
          present()
          return
        }
        case 'jpeg': {
          if (!screen) return
          // Декодирует браузер: он это делает быстрее, чем декодер, который пришлось бы
          // тащить в Rust ради того же результата.
          const blob = new Blob([f.bytes], { type: 'image/jpeg' })
          void createImageBitmap(blob).then((bmp) => {
            screen.ctx.drawImage(bmp, f.rect.x, f.rect.y)
            bmp.close()
            present()
          })
          return
        }
        case 'copy': {
          if (!screen) return
          screen.ctx.drawImage(
            screen.canvas,
            f.src.x,
            f.src.y,
            f.dst.w,
            f.dst.h,
            f.dst.x,
            f.dst.y,
            f.dst.w,
            f.dst.h
          )
          present()
          return
        }
        case 'closed': {
          setStatus('closed')
          if (f.reason) setError(f.reason)
          // Форму пароля показываем только когда дело в нём: в остальных случаях она
          // сбивает с толку, потому что проблема не там.
          setNeedPassword(f.needsPassword)
          setBlocked(f.blacklisted)
          idRef.current = null
          return
        }
        case 'text': {
          // Скопировали на сервере - значит текст должен быть доступен и здесь.
          if (f.text) void window.api.clipboard.write(f.text)
          return
        }
        // Звонок пока пропускаем молча.
        default:
          return
      }
    },
    [present]
  )

  const connect = useCallback(
    async (secret?: string) => {
      setStatus('connecting')
      setError('')
      try {
        idRef.current = await window.api.vnc.open(sessionId, draw, { password: secret })
        // Пароль дальше не нужен: он ушёл на сервер, а в состоянии панели остался бы
        // висеть всё время сеанса - в памяти окна и в снимках состояния.
        setPassword('')
      } catch (e) {
        setStatus('closed')
        setError(errText(e))
        // Рукопожатие падает до первого кадра, поэтому отказ по паролю приходит сюда, а не
        // пакетом закрытия. Признак - поле рядом с текстом, а не разбор самого текста.
        const err = e as { needsPassword?: boolean; blacklisted?: boolean } | null
        setNeedPassword(!!err?.needsPassword)
        setBlocked(!!err?.blacklisted)
      }
    },
    [sessionId, draw]
  )

  // Либо подхватываем уже открытый стол, либо подключаемся сами. Подхват просит сервер
  // перерисовать экран целиком: RFB присылает только изменения, и новое окно осталось бы
  // с пустым холстом до первого движения на сервере.
  useEffect(() => {
    if (existingId) {
      idRef.current = existingId
      setStatus('connecting')
      window.api.vnc.attach(existingId, draw).catch((e) => {
        idRef.current = null
        setStatus('closed')
        setError(errText(e))
      })
      return
    }
    void connect()
    // `draw` меняется вместе с масштабом, а подхватывать сеанс второй раз нельзя:
    // повторный вызов отнял бы кадры у самого себя.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connect, existingId])

  useEffect(() => {
    const onResize = (): void => present()
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [present])

  useEffect(() => present(), [present, scaled])

  // Вкладку показали снова: пока она была скрыта, у холста был нулевой размер, и вписать
  // картинку было не во что. Следующего кадра от неподвижного экрана можно ждать долго.
  useEffect(() => {
    if (visible) present()
  }, [visible, present])

  /*
   * Указатель уходит на каждое движение мыши, поэтому отказ здесь глушится намеренно.
   * Показывать его негде и незачем: если канал отвалился, об этом скажет сам разрыв
   * отдельным кадром, а не шестьдесят одинаковых ошибок в секунду.
   */
  const sendPointer = (id: string, x: number, y: number, buttons: number): void => {
    window.api.vnc.pointer(id, x, y, buttons).catch(() => {})
  }

  const send = (e: React.MouseEvent, buttons: number): void => {
    const id = idRef.current
    const p = toRemote(e)
    if (!id || !p) return
    sendPointer(id, p.x, p.y, buttons)
  }

  /** Вставка локального буфера на сервер по Ctrl+V. */
  const paste = async (): Promise<void> => {
    const id = idRef.current
    if (!id) return
    const text = await window.api.clipboard.read()
    if (text) await window.api.vnc.paste(id, text)
  }

  const onKey = (e: React.KeyboardEvent, down: boolean): void => {
    const id = idRef.current
    if (!id) return

    // Ctrl+V перехватываем у сервера: пользователь ждёт свой буфер, а не серверный.
    // Ctrl+Shift+V - тоже, это привычное сочетание для терминалов.
    if (down && (e.ctrlKey || e.metaKey) && (e.key === 'v' || e.key === 'V' || e.key === 'м' || e.key === 'М')) {
      e.preventDefault()
      void paste()
      return
    }

    const sym = keysymFor(e.nativeEvent)
    if (sym === null) return
    // Иначе Tab уводит фокус, а Ctrl+W закрывает вкладку вместо ухода на сервер.
    e.preventDefault()
    // Авто-повтор модификатора не несёт смысла: посреди сочетания вроде Alt+Shift он
    // читается сервером как ещё одно переключение раскладки.
    if (down && e.nativeEvent.repeat && isModifier(e.nativeEvent.code)) return
    if (down) heldRef.current.add(sym)
    else heldRef.current.delete(sym)
    window.api.vnc.key(id, sym, down).catch(() => {})
  }

  /** Отпускает всё удерживаемое. Зовётся при потере фокуса холстом. */
  const releaseHeld = (): void => {
    const id = idRef.current
    for (const sym of heldRef.current) {
      if (id) window.api.vnc.key(id, sym, false).catch(() => {})
    }
    heldRef.current.clear()
  }

  const detach = async (): Promise<void> => {
    if (!panelTitle) return
    try {
      await openDetachedWorkspace({ tool: 'desktop', sessionId, title: panelTitle })
      onDetached?.()
    } catch (e) {
      // Отказ здесь раньше уходил в общий обработчик и не доходил до человека: снаружи
      // это выглядело как «кнопка не работает». Теперь причина видна в самой панели.
      setError(`Не удалось открепить окно: ${errText(e)}`)
    }
  }

  return (
    <div className={'ws-panel vnc-panel' + (fill ? ' fill' : '')} ref={rootRef}>
      <div className="ws-head">
        <span className="ws-head-title">
          <Icon name="desktop" size={15} /> Рабочий стол
          <span className="vnc-status">
            {status === 'connecting' && 'подключение…'}
            {status === 'live' && `${sizeRef.current.w}×${sizeRef.current.h}`}
            {status === 'closed' && 'нет связи'}
          </span>
        </span>
        <div style={{ display: 'flex', gap: 6 }}>
          {onBack && (
            <button
              className="mini"
              title="Отключиться и выбрать способ подключения"
              onClick={() => {
                // Сеанс закрывается только здесь, по явному действию. Уход на другую
                // вкладку или закрытие окна его не трогают: он принадлежит SSH-сессии, а
                // не окну, и второе окно может рисовать тот же стол.
                const id = idRef.current
                idRef.current = null
                if (id) void window.api.vnc.close(id)
                onBack()
              }}
            >
              <Icon name="back" size={14} />
            </button>
          )}
          {onSetup && (
            <button className="mini" title="Настроить VNC на сервере" onClick={onSetup}>
              <Icon name="settings" size={14} />
            </button>
          )}
          {panelTitle && onDetached && <WsDetachButton onClick={detach} />}
          <button
            className={'mini' + (scaled ? ' on' : '')}
            title={scaled ? 'Показать один к одному' : 'Вписать в окно'}
            onClick={() => setScaled((v) => !v)}
          >
            <Icon name={scaled ? 'win-restore' : 'win-max'} size={14} />
          </button>
          <button
            className="mini"
            title="Перерисовать экран целиком"
            onClick={() => idRef.current && void window.api.vnc.refresh(idRef.current, true)}
          >
            <Icon name="refresh" size={14} />
          </button>
          <button
            className={'mini' + (full ? ' on' : '')}
            title={full ? 'Выйти из полного экрана (Esc)' : 'На весь экран'}
            onClick={() => {
              toggleFull()
              // Фокус холсту: иначе клавиши уйдут кнопке, а не рабочему столу.
              viewRef.current?.focus()
            }}
          >
            <Icon name={full ? 'collapse' : 'expand'} size={14} />
          </button>
        </div>
      </div>

      <div className="vnc-view">
        <canvas
          ref={viewRef}
          tabIndex={0}
          className="vnc-canvas"
          onMouseMove={(e) => send(e, buttonMask(e.buttons))}
          onMouseDown={(e) => {
            ;(e.currentTarget as HTMLCanvasElement).focus()
            send(e, buttonMask(e.buttons))
          }}
          onMouseUp={(e) => send(e, buttonMask(e.buttons))}
          onContextMenu={(e) => e.preventDefault()}
          onWheel={(e) => {
            const id = idRef.current
            const p = toRemote(e)
            if (!id || !p) return
            // В RFB прокрутка - это нажатие и отпускание кнопки, отдельного события нет.
            const mask = wheelMask(e.deltaY, e.deltaX)
            sendPointer(id, p.x, p.y, mask)
            sendPointer(id, p.x, p.y, 0)
          }}
          onKeyDown={(e) => onKey(e, true)}
          onKeyUp={(e) => onKey(e, false)}
          onBlur={releaseHeld}
        />

        {status !== 'live' && (
          <div className="vnc-overlay">
            {status === 'connecting' && <div>Подключение к рабочему столу…</div>}
            {status === 'closed' && (
              <>
                <div className="vnc-error">{error || 'Соединение закрыто'}</div>
                {/* При блокировке форму не показываем вовсе: она подсказывала бы
                    попробовать ещё раз, а каждая попытка блокировку продлевает. */}
                {blocked && (
                  <div className="vnc-hint">
                    Сервер временно закрыл доступ после неудачных попыток входа. Снимается
                    перезапуском службы: <code>systemctl restart vncserver@:1</code>
                  </div>
                )}
                {needPassword && (
                  <form
                    className="vnc-auth"
                    onSubmit={(e) => {
                      e.preventDefault()
                      void connect(password)
                    }}
                  >
                    <input
                      type="password"
                      value={password}
                      autoFocus
                      placeholder="Пароль VNC"
                      onChange={(e) => setPassword(e.target.value)}
                    />
                    <button className="primary" type="submit">
                      Подключиться
                    </button>
                  </form>
                )}
                {!needPassword && (
                  <div className="vnc-actions">
                    <button className="secondary" onClick={() => void connect(password || undefined)}>
                      Повторить
                    </button>
                    {/* Если VNC на сервере нет, повторять нечего - там и правда никого
                        нет. Полезнее увести туда, где его можно поставить. */}
                    {onSetup && (
                      <button className="secondary" onClick={onSetup}>
                        <Icon name="settings" size={13} /> Настроить сервер
                      </button>
                    )}
                  </div>
                )}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
