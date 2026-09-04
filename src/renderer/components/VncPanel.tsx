import { useCallback, useEffect, useRef, useState } from 'react'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'
import { errText } from '../errText'
import { parseFrame } from '../vncFrames'
import { buttonMask, keysymFor, wheelMask } from '../vncKeys'

/**
 * Рабочий стол VNC внутри вкладки сервера.
 *
 * Идёт через уже открытую SSH-сессию, а не отдельным соединением: VNC на сервере обычно
 * слушает `127.0.0.1`, и это правильно — его собственная защита слабая (пароль до восьми
 * символов на DES), выставлять её в сеть незачем. Разбор кадров и клавиш живёт в
 * `vncFrames.ts` и `vncKeys.ts` — там же тесты.
 */

interface Props {
  sessionId: string
  panelTitle?: string
  onDetached?: () => void
  /** В откреплённом окне панель занимает его целиком. */
  fill?: boolean
}

/** Экран рисуется в свой холст и оттуда копируется на видимый — так проще с масштабом. */
interface Screen {
  canvas: HTMLCanvasElement
  ctx: CanvasRenderingContext2D
}

export function VncPanel({ sessionId, panelTitle, onDetached, fill }: Props): JSX.Element {
  const viewRef = useRef<HTMLCanvasElement | null>(null)
  const screenRef = useRef<Screen | null>(null)
  const idRef = useRef<string | null>(null)
  const sizeRef = useRef<{ w: number; h: number }>({ w: 0, h: 0 })

  const [status, setStatus] = useState<'connecting' | 'live' | 'closed'>('connecting')
  const [error, setError] = useState('')
  const [password, setPassword] = useState('')
  const [needPassword, setNeedPassword] = useState(false)
  const [scaled, setScaled] = useState(true)

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
          idRef.current = null
          return
        }
        case 'text': {
          // Скопировали на сервере — значит текст должен быть доступен и здесь.
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
      } catch (e) {
        setStatus('closed')
        setError(errText(e))
        // Рукопожатие падает до первого кадра, поэтому отказ по паролю приходит сюда, а не
        // пакетом закрытия. Признак — поле рядом с текстом, а не разбор самого текста.
        setNeedPassword(!!(e as { needsPassword?: boolean } | null)?.needsPassword)
      }
    },
    [sessionId, draw]
  )

  useEffect(() => {
    void connect()
    return () => {
      const id = idRef.current
      idRef.current = null
      if (id) void window.api.vnc.close(id)
    }
  }, [connect])

  useEffect(() => {
    const onResize = (): void => present()
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [present])

  useEffect(() => present(), [present, scaled])

  const send = (e: React.MouseEvent, buttons: number): void => {
    const id = idRef.current
    const p = toRemote(e)
    if (!id || !p) return
    void window.api.vnc.pointer(id, p.x, p.y, buttons)
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
    // Ctrl+Shift+V — тоже, это привычное сочетание для терминалов.
    if (down && (e.ctrlKey || e.metaKey) && (e.key === 'v' || e.key === 'V' || e.key === 'м' || e.key === 'М')) {
      e.preventDefault()
      void paste()
      return
    }

    const sym = keysymFor(e.nativeEvent)
    if (sym === null) return
    // Иначе Tab уводит фокус, а Ctrl+W закрывает вкладку вместо ухода на сервер.
    e.preventDefault()
    void window.api.vnc.key(id, sym, down)
  }

  const detach = async (): Promise<void> => {
    if (!panelTitle) return
    await openDetachedWorkspace({ tool: 'desktop', sessionId, title: panelTitle })
    onDetached?.()
  }

  return (
    <div className={'ws-panel vnc-panel' + (fill ? ' fill' : '')}>
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
            // В RFB прокрутка — это нажатие и отпускание кнопки, отдельного события нет.
            const mask = wheelMask(e.deltaY, e.deltaX)
            void window.api.vnc.pointer(id, p.x, p.y, mask)
            void window.api.vnc.pointer(id, p.x, p.y, 0)
          }}
          onKeyDown={(e) => onKey(e, true)}
          onKeyUp={(e) => onKey(e, false)}
        />

        {status !== 'live' && (
          <div className="vnc-overlay">
            {status === 'connecting' && <div>Подключение к рабочему столу…</div>}
            {status === 'closed' && (
              <>
                <div className="vnc-error">{error || 'Соединение закрыто'}</div>
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
                  <button className="secondary" onClick={() => void connect(password || undefined)}>
                    Повторить
                  </button>
                )}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
