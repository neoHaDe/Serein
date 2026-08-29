import { useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { PhysicalPosition } from '@tauri-apps/api/dpi'
import { emit, listen } from '@tauri-apps/api/event'
import { getCurrentWindow, type Window } from '@tauri-apps/api/window'
import { getAllWebviewWindows } from '@tauri-apps/api/webviewWindow'
import { appPlatform } from './platform'

const MAGNET = 22
const FLUSH = 4
/** Пауза без движения, после которой считаем перетаскивание законченным. */
const SETTLE = 220
/** Столько тишины — и следующее движение считается новым жестом (группа пересобирается). */
const GESTURE_GAP = 320
/** На столько глушим собственные события после того, как двигали окно сами. */
const SKIP = 400
/** Столько пикселей руками — и доп. окно считается выведенным из группы. */
const DETACH_SLOP = 6
/** Не чаще чем раз в столько мс сообщаем главному окну, куда переехали. */
const GEO_STEP = 60

export type Box = {
  label: string
  x: number
  y: number
  w: number
  h: number
}

type R = Box & {
  toOuter: (x: number, y: number) => { x: number; y: number }
}

type GroupMove = { origin: string; dx: number; dy: number; members: string[] }

function overlap(a1: number, a2: number, b1: number, b2: number, slop: number): boolean {
  return a1 < b2 + slop && b1 < a2 + slop
}

export function flush(a: Box, b: Box): boolean {
  const yOver = overlap(a.y, a.y + a.h, b.y, b.y + b.h, FLUSH)
  const xOver = overlap(a.x, a.x + a.w, b.x, b.x + b.w, FLUSH)
  if (yOver && Math.abs(a.x + a.w - b.x) <= FLUSH) return true
  if (yOver && Math.abs(b.x + b.w - a.x) <= FLUSH) return true
  if (xOver && Math.abs(a.y + a.h - b.y) <= FLUSH) return true
  if (xOver && Math.abs(b.y + b.h - a.y) <= FLUSH) return true
  return false
}

function nearest(pos: number, size: number, others: { p: number; s: number }[]): number {
  let best = pos
  let bestD = MAGNET + 1
  const take = (target: number): void => {
    const d = Math.abs(pos - target)
    if (d <= MAGNET && d < bestD) {
      bestD = d
      best = target
    }
  }
  for (const o of others) {
    take(o.p)
    take(o.p + o.s - size)
    take(o.p - size)
    take(o.p + o.s)
    take(o.p + o.s / 2 - size / 2)
  }
  return best
}

/** Photoshop-style: X и Y магнитятся независимо (стык, края, центр). */
export function snapFlush(moving: Box, others: Box[]): { x: number; y: number } | null {
  const nx = nearest(
    moving.x,
    moving.w,
    others.map((o) => ({ p: o.x, s: o.w }))
  )
  const ny = nearest(
    moving.y,
    moving.h,
    others.map((o) => ({ p: o.y, s: o.h }))
  )
  if (Math.abs(nx - moving.x) < 1 && Math.abs(ny - moving.y) < 1) return null
  return { x: nx, y: ny }
}

export function flood(seed: Box, all: Box[]): Set<string> {
  const by = new Map(all.map((r) => [r.label, r]))
  by.set(seed.label, seed)
  const g = new Set<string>([seed.label])
  let grew = true
  while (grew) {
    grew = false
    for (const r of by.values()) {
      if (g.has(r.label)) continue
      for (const id of g) {
        const o = by.get(id)
        if (o && flush(r, o)) {
          g.add(r.label)
          grew = true
          break
        }
      }
    }
  }
  return g
}

async function visOf(w: Window): Promise<R> {
  const outer = await w.outerPosition()
  const inner = await w.innerPosition()
  const innerSize = await w.innerSize()
  const padX = inner.x - outer.x
  const padY = inner.y - outer.y
  return {
    label: w.label,
    x: inner.x,
    y: inner.y,
    w: innerSize.width,
    h: innerSize.height,
    toOuter: (x, y) => ({ x: x - padX, y: y - padY })
  }
}

/** Метки живых окон. Отдельно от геометрии: запрос размеров может не ответить
 *  на секунду, и считать из-за этого окно закрытым — значит выбросить его из группы. */
async function liveLabels(): Promise<Set<string>> {
  try {
    return new Set((await getAllWebviewWindows()).map((w) => w.label))
  } catch {
    return new Set()
  }
}

async function listVis(except?: string): Promise<R[]> {
  const all = await getAllWebviewWindows()
  const out: R[] = []
  for (const w of all) {
    if (except && w.label === except) continue
    try {
      out.push(await visOf(w))
    } catch {
      /* закрылось */
    }
  }
  return out
}

function emitAuxGeo(label: string, g: { x: number; y: number; w: number; h: number }): void {
  if (label === 'main') return
  void emit('serein-aux-geo', { label, ...g })
}

async function setVisPos(me: Window, vis: R, x: number, y: number): Promise<void> {
  const p = vis.toOuter(x, y)
  await me.setPosition(new PhysicalPosition(Math.round(p.x), Math.round(p.y)))
}

/**
 * Запускает `fn` не более чем в одном экземпляре. События перемещения приходят
 * десятками в секунду, и очередь вида `chain = chain.then(...)` копила бы работу,
 * которая продолжает двигать окна ещё долго после того, как мышь отпустили.
 * Здесь же, сколько бы событий ни пришло, в полёте всегда ровно один проход,
 * а всё пришедшее за время прохода сворачивается в один повтор.
 */
function coalesced(fn: () => Promise<void>): () => void {
  let running = false
  let pending = false
  const run = (): void => {
    if (running) {
      pending = true
      return
    }
    running = true
    void fn()
      .catch(() => {
        /* окно могло закрыться посреди работы */
      })
      .finally(() => {
        running = false
        if (pending) {
          pending = false
          run()
        }
      })
  }
  return run
}

let lastRaise = 0
let raiseBusy = false
let suppressRestoreUntil = 0

/** Подавить restore/raise на время minimize (фокус уходит на другое окно; контексты webview разные). */
export async function notifyWindowMinimize(): Promise<void> {
  const until = Date.now() + 1200
  suppressRestoreUntil = until
  await emit('serein-suppress-restore', { until })
}

function raiseGroup(origin: string): void {
  const now = Date.now()
  if (raiseBusy || now - lastRaise < 400 || now < suppressRestoreUntil) return
  lastRaise = now
  raiseBusy = true
  void invoke('windows_raise_group', { focused: origin }).finally(() => {
    lastRaise = Date.now()
    raiseBusy = false
  })
}

/** Стыковка видимых рамок вплотную. Группу тащит только главное окно; доп. окно можно отцепить. */
export function useWindowSnap(): void {
  useEffect(() => {
    let stopMoved: (() => void) | undefined
    let stopResized: (() => void) | undefined
    let stopListen: (() => void) | undefined
    let stopDetach: (() => void) | undefined
    let stopFocus: (() => void) | undefined
    let stopSuppress: (() => void) | undefined
    let stopGeo: (() => void) | undefined
    let disposed = false
    let skipUntil = 0
    let settle: number | undefined
    const sticky = new Set<string>()
    const offset = new Map<string, { dx: number; dy: number }>()
    /**
     * Где сейчас стоят соседние окна. Наполняется их же событиями `serein-aux-geo`,
     * которые они и так шлют при каждом перемещении. Раньше состав группы считался
     * запросом в бэкенд прямо в момент старта перетаскивания — ответ приходил через
     * несколько кадров, первые шаги уезжали без группы, и на коротких движениях
     * окна не успевали прицепиться вовсе. Здесь же всё готово заранее.
     */
    const geo = new Map<string, Box>()

    void (async () => {
      try {
        const me = getCurrentWindow()
        const isMain = me.label === 'main'
        // Linux: setPosition из чужого webview ненадёжен — двигаем группу из Rust.
        // Windows: частые invoke + SetWindowPos при перетаскивании валят процесс
        // (см. RELEASE_NOTES_v1.2.3) — aux двигают сами по emit.
        const useRustGroupMove = (await appPlatform()) === 'linux'

        // Кэш геометрии. Раньше на каждое событие перемещения уходило три запроса
        // в бэкенд (outerPosition + innerPosition + innerSize) — при перетаскивании
        // это сотни запросов в секунду на окно, и именно они забивали очередь.
        // Рамка и размер между событиями не меняются, поэтому держим их у себя.
        let pad = { x: 0, y: 0 }
        let size = { w: 0, h: 0 }
        let vis = { x: 0, y: 0 }
        let ready = false
        /**
         * Куда окно двигали мы сами. Отличать своё перемещение от пользовательского
         * по таймеру нельзя: событие от системы может прийти позже, чем истечёт
         * окно тишины, и тогда окно «отлипает» само по себе. Сравнение с заданной
         * позицией однозначно: совпало — двигали мы.
         */
        let wantPos: { x: number; y: number } | null = null

        const rOf = (): R => ({
          label: me.label,
          x: vis.x,
          y: vis.y,
          w: size.w,
          h: size.h,
          toOuter: (x, y) => ({ x: x - pad.x, y: y - pad.y })
        })

        const refresh = async (): Promise<void> => {
          const v = await visOf(me)
          vis = { x: v.x, y: v.y }
          size = { w: v.w, h: v.h }
          const outer = v.toOuter(v.x, v.y)
          pad = { x: v.x - outer.x, y: v.y - outer.y }
          ready = true
        }
        await refresh()

        // ——— перенос группы за главным окном ————————————————————————————
        let pendDx = 0
        let pendDy = 0
        let pendMembers: string[] = []
        const applyGroupMove = coalesced(async () => {
          const dx = pendDx
          const dy = pendDy
          const members = pendMembers
          pendDx = 0
          pendDy = 0
          pendMembers = []
          if (disposed || (dx === 0 && dy === 0)) return

          if (useRustGroupMove) {
            if (!isMain || !members.length) return
            skipUntil = Date.now() + SKIP
            await emit('serein-dock-move', {
              origin: me.label,
              dx,
              dy,
              members
            } satisfies GroupMove)
            await invoke('windows_nudge_group', { members, dx, dy })
            return
          }

          // Windows: ведомое окно двигает себя само — без IPC на каждый шаг.
          if (isMain) return
          skipUntil = Date.now() + SKIP
          const r = rOf()
          const nx = r.x + dx
          const ny = r.y + dy
          wantPos = { x: nx, y: ny }
          await setVisPos(me, r, nx, ny)
          vis = { x: nx, y: ny }
          emitAuxGeo(me.label, { x: nx, y: ny, w: size.w, h: size.h })
        })

        stopListen = await listen<GroupMove>('serein-dock-move', (e) => {
          const p = e.payload
          if (!p || p.origin === me.label || !ready) return
          if (!p.members.includes(me.label)) return
          if (p.dx === 0 && p.dy === 0) return
          skipUntil = Date.now() + SKIP
          if (useRustGroupMove) {
            wantPos = { x: vis.x + p.dx, y: vis.y + p.dy }
            vis = { x: wantPos.x, y: wantPos.y }
            emitAuxGeo(me.label, { x: vis.x, y: vis.y, w: size.w, h: size.h })
            return
          }
          pendDx += p.dx
          pendDy += p.dy
          applyGroupMove()
        })

        if (isMain) {
          stopDetach = await listen<{ label: string }>('serein-dock-detach', (e) => {
            const id = e.payload?.label
            if (!id) return
            sticky.delete(id)
            offset.delete(id)
          })
          stopGeo = await listen<Box>('serein-aux-geo', (e) => {
            const g = e.payload
            if (!g?.label) return
            geo.set(g.label, { label: g.label, x: g.x, y: g.y, w: g.w, h: g.h })
          })
          // Окна, открытые до нас, ещё ничего не присылали — спрашиваем один раз.
          for (const o of await listVis(me.label)) {
            geo.set(o.label, { label: o.label, x: o.x, y: o.y, w: o.w, h: o.h })
          }
        } else {
          // Представляемся сразу: иначе до первого движения нас нет в реестре
          // главного окна и в группу мы не попадём.
          emitAuxGeo(me.label, { x: vis.x, y: vis.y, w: size.w, h: size.h })
        }

        stopSuppress = await listen<{ until: number }>('serein-suppress-restore', (e) => {
          const until = e.payload?.until ?? 0
          if (until > suppressRestoreUntil) suppressRestoreUntil = until
        })

        stopFocus = await me.onFocusChanged(async (e) => {
          if (!e.payload) return
          if (Date.now() < suppressRestoreUntil) return
          try {
            if (await me.isMinimized()) return
          } catch {
            /* */
          }
          // Одна кнопка на панели задач: клик по Serein разворачивает свёрнутые aux без своей иконки.
          if (isMain) {
            await invoke('windows_restore_minimized').catch(() => {})
          }
          raiseGroup(me.label)
        })

        // ——— доводка после остановки ————————————————————————————————————
        const doSettle = coalesced(async () => {
          if (disposed) return
          await refresh()
          const cur = rOf()
          const others = await listVis(me.label)
          const live = isMain ? await liveLabels() : new Set<string>()
          if (isMain) {
            // Реестр — источник состава группы, поэтому здесь его и сверяем
            // с реальностью: закрытые окна уходят, остальные обновляются.
            for (const id of [...geo.keys()]) if (!live.has(id)) geo.delete(id)
            for (const o of others) {
              geo.set(o.label, { label: o.label, x: o.x, y: o.y, w: o.w, h: o.h })
            }
          }

          if (isMain && sticky.size) {
            // Подтягиваем отставших: за время перетаскивания окно могло не успеть
            // обработать часть шагов. Заодно выбрасываем закрытые.
            for (const id of [...sticky]) {
              const o = others.find((r) => r.label === id)
              if (!o) {
                // Из группы выбрасываем только по-настоящему закрытые окна.
                // Раньше хватало одного неответившего запроса размеров, чтобы
                // живое окно молча отцепилось.
                if (!live.has(id)) {
                  sticky.delete(id)
                  offset.delete(id)
                }
                continue
              }
              const off = offset.get(id)
              if (!off) continue
              const cdx = cur.x + off.dx - o.x
              const cdy = cur.y + off.dy - o.y
              if (Math.abs(cdx) > 2 || Math.abs(cdy) > 2) {
                await emit('serein-dock-move', {
                  origin: me.label,
                  dx: cdx,
                  dy: cdy,
                  members: [id]
                } satisfies GroupMove)
                if (useRustGroupMove) {
                  await invoke('windows_nudge_group', { members: [id], dx: cdx, dy: cdy })
                }
              }
            }
          }

          const grouped = flood(cur, others)
          if (isMain) for (const id of sticky) grouped.add(id)
          const outsiders = others.filter((o) => !grouped.has(o.label))
          const n = snapFlush(cur, outsiders)
          if (!n) {
            emitAuxGeo(me.label, { x: cur.x, y: cur.y, w: cur.w, h: cur.h })
            return
          }
          const sdx = n.x - cur.x
          const sdy = n.y - cur.y
          skipUntil = Date.now() + SKIP
          wantPos = { x: n.x, y: n.y }
          await setVisPos(me, cur, n.x, n.y)
          vis = { x: n.x, y: n.y }
          emitAuxGeo(me.label, { x: n.x, y: n.y, w: cur.w, h: cur.h })

          if (isMain) {
            const followers = [...grouped].filter((l) => l !== me.label)
            for (const id of followers) {
              sticky.add(id)
              const o = others.find((r) => r.label === id)
              if (o) offset.set(id, { dx: o.x - cur.x, dy: o.y - cur.y })
            }
            if (followers.length && (sdx !== 0 || sdy !== 0)) {
              if (useRustGroupMove) {
                pendDx += sdx
                pendDy += sdy
                for (const id of followers) {
                  if (!pendMembers.includes(id)) pendMembers.push(id)
                }
                applyGroupMove()
              } else {
                await emit('serein-dock-move', {
                  origin: me.label,
                  dx: sdx,
                  dy: sdy,
                  members: followers
                } satisfies GroupMove)
              }
            }
          }
        })

        // ——— само перемещение ————————————————————————————————————————————
        let lastMoveAt = 0
        let prev: { x: number; y: number } | null = null
        /** Где окно стояло в начале жеста. Состав группы считаем только отсюда. */
        let anchor: { x: number; y: number } | null = null
        /** Сколько пикселей окно проехало руками с начала жеста. */
        let dragAway = 0
        /** Группу в этом жесте уже двигали — состав пересобирать поздно. */
        let groupMoved = false
        let lastGeoAt = 0

        /**
         * Состав группы — синхронно, по реестру. Ищем соседей от точки, где окно
         * стояло в начале жеста: соседство проверяется по касанию рамок с допуском
         * 4 px, и от текущей позиции сосед уже не нашёлся бы.
         */
        const buildGroup = (a: { x: number; y: number }): void => {
          if (!isMain) return
          const others = [...geo.values()].filter((o) => o.label !== me.label)
          const seed: Box = { label: me.label, x: a.x, y: a.y, w: size.w, h: size.h }
          const members = flood(seed, others)
          sticky.clear()
          offset.clear()
          for (const o of others) {
            if (!members.has(o.label)) continue
            sticky.add(o.label)
            offset.set(o.label, { dx: o.x - a.x, dy: o.y - a.y })
          }
        }

        stopMoved = await me.onMoved((e) => {
          if (disposed || !ready) return
          // Позицию берём прямо из события: это внешняя рамка, внутреннюю получаем
          // из кэшированных полей. Ни одного запроса в бэкенд на шаг перетаскивания.
          const nx = e.payload.x + pad.x
          const ny = e.payload.y + pad.y
          const now = Date.now()
          // Пауза означает новый жест: состав группы мог измениться, пока мы стояли
          // (например, пользователь примагнитил ещё одно окно). Раньше группа
          // считалась один раз и больше не пересобиралась — из-за этого второе
          // доп. окно к группе уже не приклеивалось.
          if (now - lastMoveAt > GESTURE_GAP) {
            sticky.clear()
            offset.clear()
            prev = null
            anchor = { x: nx, y: ny }
            dragAway = 0
            groupMoved = false
            // Группа готова к первому же шагу — считать нечего, всё в реестре.
            buildGroup(anchor)
            // На Linux geo-события между webview иногда молчат — добираем состав
            // живым опросом. На Windows лишний listVis при старте жеста только грузит IPC.
            if (isMain && useRustGroupMove) {
              const a = anchor
              void listVis(me.label).then((others) => {
                // Если группа уже поехала — пересобирать нельзя: соседи сдвинулись,
                // а смещения считаются от точки старта жеста. Получились бы завышенные
                // смещения, и доводка после остановки растащила бы окна.
                if (disposed || !a || groupMoved) return
                for (const o of others) {
                  geo.set(o.label, { label: o.label, x: o.x, y: o.y, w: o.w, h: o.h })
                }
                buildGroup(a)
              })
            }
          }
          lastMoveAt = now

          // Своё перемещение узнаём по совпадению с заданной позицией, а окно
          // тишины оставляем как запасной признак: система могла подправить
          // координаты (край экрана, прилипание Windows).
          const ours =
            wantPos !== null && Math.abs(nx - wantPos.x) <= 3 && Math.abs(ny - wantPos.y) <= 3
          const wasSkipping = ours || now < skipUntil
          const p = prev
          prev = { x: nx, y: ny }
          vis = { x: nx, y: ny }
          if (wasSkipping) return
          // Дальше движение точно пользовательское — забываем свою цель, иначе
          // она будет глушить настоящие перемещения.
          wantPos = null
          // Главное окно собирает группу по реестру. Пусть видит нас там, где мы
          // сейчас: иначе, схватив его сразу после того, как мы примагнитились,
          // оно возьмёт нашу позицию «до» и соседа не найдёт.
          if (!isMain && now - lastGeoAt > GEO_STEP) {
            lastGeoAt = now
            emitAuxGeo(me.label, { x: nx, y: ny, w: size.w, h: size.h })
          }

          const dx = p ? nx - p.x : 0
          const dy = p ? ny - p.y : 0

          if (p && (dx !== 0 || dy !== 0)) {
            if (!isMain) {
              // Доп. окно двигают руками — оно выходит из группы. Порог нужен,
              // чтобы окно не отлипало от одиночного дрожания на пиксель.
              dragAway += Math.abs(dx) + Math.abs(dy)
              if (dragAway > DETACH_SLOP) void emit('serein-dock-detach', { label: me.label })
            } else {
              const members = [...sticky]
              if (members.length) {
                groupMoved = true
                if (useRustGroupMove) {
                  pendDx += dx
                  pendDy += dy
                  for (const id of members) {
                    if (!pendMembers.includes(id)) pendMembers.push(id)
                  }
                  applyGroupMove()
                } else {
                  void emit('serein-dock-move', {
                    origin: me.label,
                    dx,
                    dy,
                    members
                  } satisfies GroupMove)
                }
              }
            }
          }

          if (settle !== undefined) window.clearTimeout(settle)
          settle = window.setTimeout(doSettle, SETTLE)
        })

        stopResized = await me.onResized((e) => {
          if (disposed || !ready) return
          // Размер клиентской области меняется вместе с внешним; точные значения
          // подтянет доводка, а до неё хватит приблизительных — они нужны только
          // для проверки соседства.
          size = { w: e.payload.width, h: e.payload.height }
          emitAuxGeo(me.label, { x: vis.x, y: vis.y, w: size.w, h: size.h })
          if (settle !== undefined) window.clearTimeout(settle)
          settle = window.setTimeout(doSettle, SETTLE)
        })
      } catch {
        /* не Tauri */
      }
    })()

    return () => {
      disposed = true
      if (settle !== undefined) window.clearTimeout(settle)
      stopMoved?.()
      stopResized?.()
      stopListen?.()
      stopDetach?.()
      stopFocus?.()
      stopSuppress?.()
      stopGeo?.()
    }
  }, [])
}
