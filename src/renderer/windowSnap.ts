import { useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { PhysicalPosition } from '@tauri-apps/api/dpi'
import { emit, listen } from '@tauri-apps/api/event'
import { getCurrentWindow, type Window } from '@tauri-apps/api/window'
import { getAllWebviewWindows } from '@tauri-apps/api/webviewWindow'

const MAGNET = 22
const FLUSH = 4

type R = {
  label: string
  x: number
  y: number
  w: number
  h: number
  toOuter: (x: number, y: number) => { x: number; y: number }
}

type GroupMove = { origin: string; dx: number; dy: number; members: string[] }

function overlap(a1: number, a2: number, b1: number, b2: number, slop: number): boolean {
  return a1 < b2 + slop && b1 < a2 + slop
}

function flush(a: R, b: R): boolean {
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
function snapFlush(moving: R, others: R[]): { x: number; y: number } | null {
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

function flood(seed: R, all: R[]): Set<string> {
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

async function setVisPos(me: Window, vis: R, x: number, y: number): Promise<void> {
  const p = vis.toOuter(x, y)
  await me.setPosition(new PhysicalPosition(Math.round(p.x), Math.round(p.y)))
}

let lastRaise = 0
let raiseBusy = false

function raiseGroup(origin: string): void {
  const now = Date.now()
  if (raiseBusy || now - lastRaise < 400) return
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
    let stopListen: (() => void) | undefined
    let stopFocus: (() => void) | undefined
    let skipUntil = 0
    let last: { x: number; y: number; w: number; h: number } | null = null
    let settle: number | undefined
    let chain = Promise.resolve()

    void (async () => {
      try {
        const me = getCurrentWindow()

        stopListen = await listen<GroupMove>('serein-dock-move', (e) => {
          const p = e.payload
          if (!p || p.origin === me.label) return
          if (!p.members.includes(me.label)) return
          if (p.dx === 0 && p.dy === 0) return
          chain = chain.then(async () => {
            try {
              skipUntil = Date.now() + 120
              const v = await visOf(me)
              await setVisPos(me, v, v.x + p.dx, v.y + p.dy)
            } catch {
              /* */
            }
          })
        })

        stopFocus = await me.onFocusChanged((e) => {
          if (e.payload) raiseGroup(me.label)
        })

        stopMoved = await me.onMoved(() => {
          chain = chain.then(async () => {
            try {
              const v = await visOf(me)
              if (Date.now() < skipUntil) {
                last = { x: v.x, y: v.y, w: v.w, h: v.h }
                return
              }
              const prev = last
              last = { x: v.x, y: v.y, w: v.w, h: v.h }
              const dx = prev ? v.x - prev.x : 0
              const dy = prev ? v.y - prev.y : 0
              if (me.label === 'main' && prev && (dx !== 0 || dy !== 0)) {
                const others = await listVis(me.label)
                const seed: R = { ...v, x: prev.x, y: prev.y, w: prev.w, h: prev.h }
                const members = [...flood(seed, others)].filter((l) => l !== me.label)
                if (members.length) {
                  await emit('serein-dock-move', { origin: me.label, dx, dy, members } satisfies GroupMove)
                }
              }
              if (settle !== undefined) window.clearTimeout(settle)
              settle = window.setTimeout(() => {
                chain = chain.then(async () => {
                  try {
                    const cur = await visOf(me)
                    const others = await listVis(me.label)
                    const grouped = flood(cur, others)
                    const outsiders = others.filter((o) => !grouped.has(o.label))
                    const n = snapFlush(cur, outsiders)
                    if (!n) {
                      last = { x: cur.x, y: cur.y, w: cur.w, h: cur.h }
                      return
                    }
                    const sdx = n.x - cur.x
                    const sdy = n.y - cur.y
                    skipUntil = Date.now() + 120
                    await setVisPos(me, cur, n.x, n.y)
                    last = { x: n.x, y: n.y, w: cur.w, h: cur.h }
                    if (me.label === 'main') {
                      const followers = [...grouped].filter((l) => l !== me.label)
                      if (followers.length) {
                        await emit('serein-dock-move', {
                          origin: me.label,
                          dx: sdx,
                          dy: sdy,
                          members: followers
                        } satisfies GroupMove)
                      }
                    }
                  } catch {
                    /* */
                  }
                })
              }, 160)
            } catch {
              /* */
            }
          })
        })
      } catch {
        /* не Tauri */
      }
    })()

    return () => {
      if (settle !== undefined) window.clearTimeout(settle)
      stopMoved?.()
      stopListen?.()
      stopFocus?.()
    }
  }, [])
}
