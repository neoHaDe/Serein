import { useCallback, useEffect, useRef, useState } from 'react'
import { Icon } from './Icon'
import { WsDetachButton } from './WsDetachButton'
import { openDetachedWorkspace } from './workspaceWindow'
import { useVisible } from '../hooks/useVisible'
import { errText } from '../errText'
import { parseFrame } from '../vncFrames'
import { buttonMask, isModifier, isRdpShortcut, scancodeFor, wheelRotation } from '../rdpKeys'
import { RdpUiMetrics } from '../rdpMetrics'
import { useSettings } from '../SettingsContext'

/**
 * Рабочий стол по RDP.
 *
 * Отрисовка та же, что у VNC: помощник отдаёт кадры в том же формате, и разбирает их тот
 * же `parseFrame`. Отличий по существу два. Первое - вход: RDP требует имя и пароль до
 * соединения, поэтому форма показывается сразу, а не после отказа. Второе - клавиши:
 * туда уходят скан-коды физических клавиш, а не символы, и раскладку применяет сервер.
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
  onBack?: () => void
}

interface Screen {
  canvas: HTMLCanvasElement
  ctx: CanvasRenderingContext2D
}

/**
 * Готовые разрешения. Первое - не размер, а правило: следовать за окном.
 *
 * Список короткий намеренно. Перечислять все возможные размеры бессмысленно: кому нужен
 * особый, тому нужен и весь экран, а это уже «по размеру окна» на развёрнутом окне.
 */
const RESOLUTIONS = [
  { id: 'auto', name: 'По размеру окна' },
  { id: '1920x1080', name: '1920 × 1080' },
  { id: '1600x900', name: '1600 × 900' },
  { id: '1366x768', name: '1366 × 768' },
  { id: '1280x800', name: '1280 × 800' },
  { id: '1024x768', name: '1024 × 768' }
] as const

/** Через сколько после последнего движения границы просить сервер о новом размере. */
const RESIZE_DELAY = 500

export function RdpPanel({
  sessionId,
  panelTitle,
  onDetached,
  fill,
  existingId,
  onBack
}: Props): JSX.Element {
  const { settings, update } = useSettings()
  const [rootRef, visible] = useVisible<HTMLDivElement>()
  const viewRef = useRef<HTMLCanvasElement | null>(null)
  const screenRef = useRef<Screen | null>(null)
  const idRef = useRef<string | null>(null)
  const sizeRef = useRef<{ w: number; h: number }>({ w: 0, h: 0 })
  const scaledRef = useRef(true)
  const presentRafRef = useRef<number | null>(null)
  const pointerRafRef = useRef<number | null>(null)
  const pendingPointerRef = useRef<{ x: number; y: number; buttons: number } | null>(null)
  const wheelRafRef = useRef<number | null>(null)
  const pendingWheelRef = useRef<{ vertical: number; horizontal: number }>({ vertical: 0, horizontal: 0 })
  // JPEG декодируется асинхронно. Цепочка сохраняет порядок с соседними raw-кадрами:
  // иначе старый большой кадр мог дорисоваться поверх более свежего маленького.
  const paintChainRef = useRef<Promise<void>>(Promise.resolve())
  const metricsRef = useRef(new RdpUiMetrics(performance.now()))
  // Счётчик принятого и виденные виды кадров: без них разбор пустого экрана сводится
  // к разглядыванию снимков, а сказать «кадры не дошли» или «дошли, но не нарисовались»
  // невозможно. Отчёт уходит в журнал рабочего стола, рядом со строчками из Rust.
  const seenRef = useRef<{ n: number; kinds: Set<string> }>({ n: 0, kinds: new Set() })

  const [status, setStatus] = useState<'form' | 'connecting' | 'live' | 'closed'>('form')
  const [error, setError] = useState('')
  const [user, setUser] = useState('')
  const [password, setPassword] = useState('')
  const [domain, setDomain] = useState('')
  const [scaled, setScaled] = useState(true)
  const [resolution, setResolution] = useState<string>('auto')
  const [autologon, setAutologon] = useState(true)
  const depth = settings.rdpColorDepth ?? 16
  const economy = settings.rdpEconomy ?? true
  const networkProfile = settings.rdpNetworkProfile ?? 'vpn'
  const captureShortcuts = settings.rdpCaptureShortcuts !== false
  // Последний размер, о котором просили сервер. Без него наблюдатель за размером слал бы
  // просьбу и на собственный ответ сервера - тот ведь тоже меняет размер холста.
  const askedRef = useRef<{ w: number; h: number }>({ w: 0, h: 0 })
  // Что мы считаем нажатым на той стороне. Нужно, чтобы при потере фокуса отпустить всё:
  // залипший на сервере Alt довершает сочетание сам собой в самый неподходящий момент.
  const heldRef = useRef<Set<number>>(new Set())

  const reportMetrics = useCallback((force = false) => {
    const now = performance.now()
    const metrics = metricsRef.current
    if (!force && !metrics.due(now)) return
    const view = viewRef.current
    const shown = document.visibilityState === 'visible' && !!view && view.clientWidth > 0 && view.clientHeight > 0
    window.api.rdp
      .note(`метрики UI ${JSON.stringify({ ...metrics.take(now), shown })}`)
      .catch(() => {})
  }, [])

  const present = useCallback(() => {
    const started = performance.now()
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
    const pw = Math.max(1, Math.floor(cw * dpr))
    const ph = Math.max(1, Math.floor(ch * dpr))
    // Присваивание width/height очищает canvas и заново выделяет его буфер. Делать это
    // на каждом RDP-кадре дорого; меняем размер только когда окно и правда изменилось.
    if (view.width !== pw) view.width = pw
    if (view.height !== ph) view.height = ph
    const cssW = `${cw}px`
    const cssH = `${ch}px`
    if (view.style.width !== cssW) view.style.width = cssW
    if (view.style.height !== cssH) view.style.height = cssH

    const ctx = view.getContext('2d')
    if (!ctx) return
    ctx.imageSmoothingEnabled = false
    ctx.clearRect(0, 0, view.width, view.height)

    const k = scaledRef.current ? Math.min((cw * dpr) / w, (ch * dpr) / h) : dpr
    const dw = w * k
    const dh = h * k
    ctx.drawImage(screen.canvas, (view.width - dw) / 2, (view.height - dh) / 2, dw, dh)
    metricsRef.current.notePresent(performance.now() - started)
    reportMetrics()
  }, [reportMetrics])

  // Несколько прямоугольников одного серверного кадра приходят отдельными пакетами.
  // В framebuffer кладём каждый, а на экран выводим один раз за кадр браузера.
  const schedulePresent = useCallback(() => {
    if (presentRafRef.current !== null) return
    presentRafRef.current = requestAnimationFrame(() => {
      presentRafRef.current = null
      if (document.visibilityState === 'visible') present()
    })
  }, [present])

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
      const seen = seenRef.current
      seen.n += 1
      const decodeStarted = performance.now()
      const f = parseFrame(buf)
      metricsRef.current.noteFrame(buf.byteLength, performance.now() - decodeStarted)
      if (!f) {
        window.api.rdp.note(`кадр ${seen.n}: не разобрался, ${buf.byteLength} Б`).catch(() => {})
        return
      }
      if (!seen.kinds.has(f.kind)) {
        seen.kinds.add(f.kind)
        const size = f.kind === 'raw' ? ` ${f.rect.w}×${f.rect.h} в ${f.rect.x},${f.rect.y}` : ''
        window.api.rdp.note(`кадр ${seen.n}: впервые вид «${f.kind}»${size}`).catch(() => {})
      }
      const screen = screenRef.current

      switch (f.kind) {
        case 'resize': {
          sizeRef.current = { w: f.w, h: f.h }
          const canvas = document.createElement('canvas')
          canvas.width = f.w
          canvas.height = f.h
          const ctx = canvas.getContext('2d')
          if (!ctx) return
          if (screen) ctx.drawImage(screen.canvas, 0, 0)
          screenRef.current = { canvas, ctx }
          setStatus('live')
          schedulePresent()
          // Фокус сразу на холст: иначе клавиатура молчит до первого клика, и это
          // читается как «ввод не работает», а не «нажмите сюда».
          viewRef.current?.focus()
          return
        }
        case 'raw': {
          if (!screen) {
            window.api.rdp.note(`кадр ${seen.n}: холста ещё нет, рисовать некуда`).catch(() => {})
            return
          }
          paintChainRef.current = paintChainRef.current
            .then(() => {
              const drawStarted = performance.now()
              screen.ctx.putImageData(new ImageData(f.pixels, f.rect.w, f.rect.h), f.rect.x, f.rect.y)
              metricsRef.current.noteRaw(f.rect.w * f.rect.h, performance.now() - drawStarted)
              schedulePresent()
            })
            .catch((e) => {
              const why = errText(e)
              window.api.rdp.note(`кадр ${seen.n}: не нарисовался - ${why}`).catch(() => {})
              setError(`Кадр не нарисовался: ${why}`)
            })
          return
        }
        case 'jpeg': {
          if (!screen) return
          const bytes = f.bytes
          const rect = f.rect
          paintChainRef.current = paintChainRef.current
            .then(async () => {
              const blob = new Blob([bytes], { type: 'image/jpeg' })
              const bitmap = await createImageBitmap(blob)
              screen.ctx.drawImage(bitmap, rect.x, rect.y)
              bitmap.close()
              schedulePresent()
            })
            .catch((e) => {
              const why = errText(e)
              window.api.rdp.note(`JPEG-кадр ${seen.n}: не нарисовался - ${why}`).catch(() => {})
              setError(`Кадр не нарисовался: ${why}`)
            })
          return
        }
        case 'closed': {
          reportMetrics(true)
          setStatus('closed')
          if (f.reason) setError(f.reason)
          idRef.current = null
          return
        }
        default:
          return
      }
    },
    [reportMetrics, schedulePresent]
  )

  /** Размер, с которым подключаться: либо выбранный числом, либо нынешний размер окна. */
  const wantedSize = useCallback((): { w: number; h: number } | null => {
    if (resolution !== 'auto') {
      const [w, h] = resolution.split('x').map(Number)
      return { w, h }
    }
    const box = viewRef.current?.parentElement
    if (!box) return null
    return { w: Math.floor(box.clientWidth), h: Math.floor(box.clientHeight) }
  }, [resolution])

  const connect = async (): Promise<void> => {
    setStatus('connecting')
    setError('')
    try {
      const size = wantedSize()
      if (size) askedRef.current = size
      metricsRef.current = new RdpUiMetrics(performance.now())
      window.api.rdp
        .note(
          `профиль ${JSON.stringify({ resolution, width: size?.w, height: size?.h, depth, economy, autologon, networkProfile })}`
        )
        .catch(() => {})
      idRef.current = await window.api.rdp.open(sessionId, draw, {
        user,
        password,
        domain: domain || undefined,
        width: size?.w,
        height: size?.h,
        colorDepth: depth,
        economy,
        autologon,
        networkProfile
      })
      setPassword('')
    } catch (e) {
      setStatus('closed')
      setError(errText(e))
    }
  }

  // Подхват уже открытого стола. Помощник в ответ пришлёт размер и весь экран целиком:
  // неподвижный рабочий стол сам по себе не шлёт ничего, и без этого холст остался бы
  // пустым до первого движения на сервере.
  useEffect(() => {
    if (!existingId) return
    idRef.current = existingId
    setStatus('connecting')
    window.api.rdp.attach(existingId, draw).catch((e) => {
      idRef.current = null
      setStatus('closed')
      setError(errText(e))
    })
    // Намеренно один раз: повторный подхват отнял бы кадры у самого себя.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [existingId])

  useEffect(() => {
    const onResize = (): void => present()
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [present])

  useEffect(
    () => () => {
      if (presentRafRef.current !== null) cancelAnimationFrame(presentRafRef.current)
      if (pointerRafRef.current !== null) cancelAnimationFrame(pointerRafRef.current)
      if (wheelRafRef.current !== null) cancelAnimationFrame(wheelRafRef.current)
    },
    []
  )

  useEffect(() => {
    scaledRef.current = scaled
    present()
  }, [present, scaled])

  // Вкладку показали снова - холст только что был нулевого размера, и картинку надо
  // вернуть самим: следующего кадра от неподвижного экрана можно ждать очень долго.
  useEffect(() => {
    if (visible) present()
  }, [visible, present])

  // Разрешение следом за окном. Просьба уходит не сразу: пока границу тянут мышью,
  // размер меняется десятки раз в секунду, а каждая просьба стоит серверу пересборки
  // сеанса. Ждём, пока человек отпустит, и только потом просим - один раз.
  useEffect(() => {
    if (status !== 'live' || resolution !== 'auto') return
    const box = viewRef.current?.parentElement
    if (!box) return

    let timer: ReturnType<typeof setTimeout> | undefined
    const ask = (): void => {
      const w = Math.floor(box.clientWidth)
      const h = Math.floor(box.clientHeight)
      // Скрытая вкладка даёт нулевой размер - просить о нём нечего.
      if (w < 200 || h < 200) return
      const was = askedRef.current
      // Мелкое дрожание в пару точек не стоит пересборки сеанса.
      if (Math.abs(was.w - w) < 8 && Math.abs(was.h - h) < 8) return
      askedRef.current = { w, h }
      const id = idRef.current
      if (id) window.api.rdp.resize(id, w, h).catch(() => {})
    }

    const ro = new ResizeObserver(() => {
      clearTimeout(timer)
      timer = setTimeout(ask, RESIZE_DELAY)
    })
    ro.observe(box)
    return () => {
      clearTimeout(timer)
      ro.disconnect()
    }
  }, [status, resolution])

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

  const sendPointer = (p: { x: number; y: number; buttons: number }): void => {
    const id = idRef.current
    if (!id) return
    // Отказ глушим намеренно: указатель уходит на каждое движение мыши, и при обрыве
    // это был бы поток одинаковых ошибок вместо одного внятного сообщения.
    window.api.rdp.pointer(id, p.x, p.y, p.buttons).catch(() => {})
  }

  const pointFromEvent = (e: React.MouseEvent): { x: number; y: number; buttons: number } | null => {
    const p = toRemote(e)
    return p ? { ...p, buttons: buttonMask(e.buttons) } : null
  }

  /** Движение коалесцируется: в очереди может быть максимум одна ещё не отправленная точка. */
  const queuePointer = (e: React.MouseEvent): void => {
    const p = pointFromEvent(e)
    if (!p) return
    pendingPointerRef.current = p
    if (pointerRafRef.current !== null) return
    pointerRafRef.current = requestAnimationFrame(() => {
      pointerRafRef.current = null
      const pending = pendingPointerRef.current
      pendingPointerRef.current = null
      if (pending) sendPointer(pending)
    })
  }

  /** Нажатия не ждут кадра UI и не обгоняют последнее положение указателя. */
  const sendPointerNow = (e: React.MouseEvent): void => {
    if (pointerRafRef.current !== null) {
      cancelAnimationFrame(pointerRafRef.current)
      pointerRafRef.current = null
    }
    const pending = pendingPointerRef.current
    pendingPointerRef.current = null
    if (pending) sendPointer(pending)
    const current = pointFromEvent(e)
    if (current) sendPointer(current)
  }

  const onWheel = (e: React.WheelEvent): void => {
    const id = idRef.current
    if (!id) return
    e.preventDefault()
    e.stopPropagation()
    const pending = pendingWheelRef.current
    pending.vertical = Math.max(-256, Math.min(255, pending.vertical + wheelRotation(e.deltaY, e.deltaMode)))
    pending.horizontal = Math.max(-256, Math.min(255, pending.horizontal + wheelRotation(e.deltaX, e.deltaMode)))
    if (wheelRafRef.current !== null) return
    wheelRafRef.current = requestAnimationFrame(() => {
      wheelRafRef.current = null
      const { vertical, horizontal } = pendingWheelRef.current
      pendingWheelRef.current = { vertical: 0, horizontal: 0 }
      const liveId = idRef.current
      if (!liveId) return
      if (vertical) window.api.rdp.wheel(liveId, true, vertical).catch(() => {})
      if (horizontal) window.api.rdp.wheel(liveId, false, horizontal).catch(() => {})
    })
  }

  const onKey = (e: React.KeyboardEvent, down: boolean): void => {
    const id = idRef.current
    if (!id) return
    const code = scancodeFor(e.nativeEvent.code)
    if (code === null) return
    // Если перехват выключен, сочетания остаются приложению. Уже отправленную клавишу
    // всё равно отпускаем: настройку могли переключить, пока она была нажата.
    if (!captureShortcuts && isRdpShortcut(e.nativeEvent) && !heldRef.current.has(code)) return
    // Иначе Tab уводит фокус, а Ctrl+W закрывает вкладку вместо ухода на сервер.
    e.preventDefault()
    e.stopPropagation()
    // Авто-повтор модификатора не несёт смысла, а вреда много: см. `isModifier`.
    if (down && e.nativeEvent.repeat && isModifier(e.nativeEvent.code)) return
    if (down) heldRef.current.add(code)
    else heldRef.current.delete(code)
    window.api.rdp.key(id, code, down).catch(() => {})
  }

  /** Отпускает всё удерживаемое. Зовётся при потере фокуса холстом. */
  const releaseHeld = useCallback((): void => {
    const id = idRef.current
    for (const code of heldRef.current) {
      if (id) window.api.rdp.key(id, code, false).catch(() => {})
    }
    heldRef.current.clear()
  }, [])

  // При Alt+Tab окно теряет фокус целиком, а не обязательно через blur самого canvas.
  // Отпускаем модификаторы и здесь, чтобы Alt/Ctrl не оставались зажатыми на сервере.
  useEffect(() => {
    window.addEventListener('blur', releaseHeld)
    return () => window.removeEventListener('blur', releaseHeld)
  }, [releaseHeld])

  return (
    <div className={'ws-panel vnc-panel' + (fill ? ' fill' : '')} ref={rootRef}>
      <div className="ws-head">
        <span className="ws-head-title">
          <Icon name="monitor" size={15} /> RDP
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
                if (id) void window.api.rdp.close(id)
                onBack()
              }}
            >
              <Icon name="back" size={14} />
            </button>
          )}
          {panelTitle && onDetached && <WsDetachButton onClick={detach} />}
          {status === 'live' && (
            <button
              className={'mini' + (captureShortcuts ? ' on' : '')}
              title={captureShortcuts ? 'Сочетания клавиш уходят в RDP' : 'Сочетания остаются Serein'}
              onClick={() => update({ rdpCaptureShortcuts: !captureShortcuts })}
            >
              <Icon name="key" size={14} />
            </button>
          )}
          {status === 'live' && (
            <button
              className="mini"
              title="Отправить Ctrl+Alt+Del на сервер"
              onClick={() => {
                const id = idRef.current
                if (id) void window.api.rdp.secureAttention(id)
              }}
            >
              CAD
            </button>
          )}
          {status === 'live' && (
            <button
              className={'mini' + (scaled ? ' on' : '')}
              title={scaled ? 'Показать один к одному' : 'Вписать в окно'}
              onClick={() => setScaled((v) => !v)}
            >
              <Icon name={scaled ? 'win-restore' : 'win-max'} size={14} />
            </button>
          )}
        </div>
      </div>

      <div className="vnc-view">
        <canvas
          ref={viewRef}
          tabIndex={0}
          className="vnc-canvas"
          data-keycapture={captureShortcuts ? '' : undefined}
          onMouseMove={queuePointer}
          onMouseDown={(e) => {
            ;(e.currentTarget as HTMLCanvasElement).focus()
            sendPointerNow(e)
          }}
          onMouseUp={sendPointerNow}
          onWheel={onWheel}
          onContextMenu={(e) => e.preventDefault()}
          onKeyDown={(e) => onKey(e, true)}
          onKeyUp={(e) => onKey(e, false)}
          onBlur={releaseHeld}
        />

        {status !== 'live' && (
          <div className="vnc-overlay">
            {status === 'connecting' && <div>Подключение к рабочему столу…</div>}
            {error && <div className="vnc-error">{error}</div>}
            {status !== 'connecting' && (
              <form
                className="rdp-auth"
                onSubmit={(e) => {
                  e.preventDefault()
                  void connect()
                }}
              >
                {/* RDP спрашивает имя до соединения, а не после отказа, как VNC:
                    без учётных данных сервер не начнёт даже рукопожатие. */}
                <label>
                  Пользователь
                  <input value={user} autoFocus onChange={(e) => setUser(e.target.value)} />
                </label>
                <label>
                  Пароль
                  <input
                    type="password"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                  />
                </label>
                <label>
                  Домен, если есть
                  <input value={domain} onChange={(e) => setDomain(e.target.value)} />
                </label>

                <div className="rdp-auth-row">
                  <label>
                    Разрешение
                    <select value={resolution} onChange={(e) => setResolution(e.target.value)}>
                      {RESOLUTIONS.map((r) => (
                        <option key={r.id} value={r.id}>
                          {r.name}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    Цвет
                    <select
                      value={depth}
                      onChange={(e) => update({ rdpColorDepth: Number(e.target.value) as 16 | 24 | 32 })}
                    >
                      <option value={32}>32 бита</option>
                      <option value={24}>24 бита</option>
                      <option value={16}>16 бит</option>
                    </select>
                  </label>
                </div>

                <label>
                  Сеть
                  <select
                    value={networkProfile}
                    onChange={(e) => update({ rdpNetworkProfile: e.target.value as 'vpn' | 'lan' })}
                  >
                    <option value="vpn">VPN / ограниченная</option>
                    <option value="lan">Быстрая локальная сеть</option>
                  </select>
                </label>

                <label className="rdp-auth-check">
                  <input
                    type="checkbox"
                    checked={economy}
                    onChange={(e) => update({ rdpEconomy: e.target.checked })}
                  />
                  Экономить трафик: без обоев, тем и анимации
                </label>

                <label className="rdp-auth-check">
                  <input
                    type="checkbox"
                    checked={captureShortcuts}
                    onChange={(e) => update({ rdpCaptureShortcuts: e.target.checked })}
                  />
                  Передавать сочетания клавиш в удалённый сеанс
                </label>

                <label className="rdp-auth-check">
                  <input
                    type="checkbox"
                    checked={autologon}
                    onChange={(e) => setAutologon(e.target.checked)}
                  />
                  Входить сразу, без окна входа на сервере
                </label>

                <button className="primary" type="submit" disabled={!user}>
                  Подключиться
                </button>
              </form>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
