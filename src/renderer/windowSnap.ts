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

function emitAuxGeo(label: string, g: { x: number; y: number; w: number; h: number }): void {
  if (label === 'main') return
  void emit('serein-aux-geo', { label, ...g })
}

async function setVisPos(me: Window, vis: R, x: number, y: number): Promise<void> {
  const p = vis.toOuter(x, y)
  await me.setPosition(new PhysicalPosition(Math.round(p.x), Math.round(p.y)))
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
    let skipUntil = 0
    let last: { x: number; y: number; w: number; h: number } | null = null
    let settle: number | undefined
    let chain = Promise.resolve()
    const sticky = new Set<string>()
    const offset = new Map<string, { dx: number; dy: number }>()
    let pendDx = 0
    let pendDy = 0

    void (async () => {
      try {
        const me = getCurrentWindow()

        stopListen = await listen<GroupMove>('serein-dock-move', (e) => {
          const p = e.payload
          if (!p || p.origin === me.label) return
          if (!p.members.includes(me.label)) return
          if (p.dx === 0 && p.dy === 0) return
          skipUntil = Date.now() + 400
          pendDx += p.dx
          pendDy += p.dy
          chain = chain.then(async () => {
            const dx = pendDx
            const dy = pendDy
            pendDx = 0
            pendDy = 0
            if (dx === 0 && dy === 0) return
            try {
              skipUntil = Date.now() + 400
              const v = await visOf(me)
              await setVisPos(me, v, v.x + dx, v.y + dy)
              last = { x: v.x + dx, y: v.y + dy, w: v.w, h: v.h }
              emitAuxGeo(me.label, last)
            } catch {
              /* */
            }
          })
        })

        if (me.label === 'main') {
          stopDetach = await listen<{ label: string }>('serein-dock-detach', (e) => {
            const id = e.payload?.label
            if (!id) return
            sticky.delete(id)
            offset.delete(id)
          })
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
          if (me.label === 'main') {
            await invoke('windows_restore_minimized').catch(() => {})
          }
          raiseGroup(me.label)
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

              if (me.label !== 'main' && prev && (dx !== 0 || dy !== 0)) {
                void emit('serein-dock-detach', { label: me.label })
              }

              if (me.label === 'main' && prev && (dx !== 0 || dy !== 0)) {
                let members = [...sticky]
                if (!members.length) {
                  const others = await listVis(me.label)
                  const seed: R = { ...v, x: prev.x, y: prev.y, w: prev.w, h: prev.h }
                  members = [...flood(seed, others)].filter((l) => l !== me.label)
                  sticky.clear()
                  offset.clear()
                  for (const o of others) {
                    if (!members.includes(o.label)) continue
                    sticky.add(o.label)
                    offset.set(o.label, { dx: o.x - prev.x, dy: o.y - prev.y })
                  }
                }
                if (members.length) {
                  void emit('serein-dock-move', { origin: me.label, dx, dy, members } satisfies GroupMove)
                }
              }
              if (settle !== undefined) window.clearTimeout(settle)
              settle = window.setTimeout(() => {
                chain = chain.then(async () => {
                  try {
                    const cur = await visOf(me)
                    const others = await listVis(me.label)
                    if (me.label === 'main' && sticky.size) {
                      for (const id of [...sticky]) {
                        const o = others.find((r) => r.label === id)
                        if (!o) {
                          sticky.delete(id)
                          offset.delete(id)
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
                        }
                      }
                    }
                    const grouped = flood(cur, others)
                    if (me.label === 'main') {
                      for (const id of sticky) grouped.add(id)
                    }
                    const outsiders = others.filter((o) => !grouped.has(o.label))
                    const n = snapFlush(cur, outsiders)
                    if (!n) {
                      last = { x: cur.x, y: cur.y, w: cur.w, h: cur.h }
                      emitAuxGeo(me.label, last)
                      return
                    }
                    const sdx = n.x - cur.x
                    const sdy = n.y - cur.y
                    skipUntil = Date.now() + 400
                    await setVisPos(me, cur, n.x, n.y)
                    last = { x: n.x, y: n.y, w: cur.w, h: cur.h }
                    emitAuxGeo(me.label, last)
                    if (me.label === 'main') {
                      const followers = [...grouped].filter((l) => l !== me.label)
                      if (followers.length) {
                        for (const id of followers) {
                          sticky.add(id)
                          const o = others.find((r) => r.label === id)
                          if (o) offset.set(id, { dx: o.x - cur.x, dy: o.y - cur.y })
                        }
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
              }, 220)
            } catch {
              /* */
            }
          })
        })

        stopResized = await me.onResized(() => {
          chain = chain.then(async () => {
            try {
              const v = await visOf(me)
              last = { x: v.x, y: v.y, w: v.w, h: v.h }
              emitAuxGeo(me.label, last)
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
      stopResized?.()
      stopListen?.()
      stopDetach?.()
      stopFocus?.()
      stopSuppress?.()
    }
  }, [])
}
