import { listen } from '@tauri-apps/api/event'
import type { AuxLayout, SavedAuxWindow } from '../shared/types'

export function auxWindowKey(w: Pick<SavedAuxWindow, 'kind' | 'serverId' | 'containerId'>): string {
  return w.kind === 'dockerLogs' ? `dockerLogs:${w.serverId}:${w.containerId ?? ''}` : `sftp:${w.serverId}`
}

let persistOn = false
let persistReady = false
let saveTimer: number | undefined
const live = new Map<string, SavedAuxWindow>()
const labelToKey = new Map<string, string>()

export function setAuxPersistEnabled(on: boolean): void {
  persistOn = on
}

export function markAuxPersistReady(): void {
  persistReady = true
}

export function seedAuxLive(windows: SavedAuxWindow[]): void {
  live.clear()
  labelToKey.clear()
  for (const w of windows) {
    if (!w?.kind || !w.serverId) continue
    live.set(auxWindowKey(w), { ...w })
  }
}

export function registerAuxWindow(label: string, meta: SavedAuxWindow): void {
  const key = auxWindowKey(meta)
  live.set(key, { ...meta })
  labelToKey.set(label, key)
  scheduleSave()
}

export function unregisterAuxWindow(label: string): void {
  const key = labelToKey.get(label)
  if (!key) return
  labelToKey.delete(label)
  live.delete(key)
  scheduleSave()
}

export function updateAuxGeometry(label: string, geo: { x: number; y: number; w: number; h: number }): void {
  const key = labelToKey.get(label)
  if (!key) return
  const cur = live.get(key)
  if (!cur) return
  live.set(key, { ...cur, ...geo })
  scheduleSave()
}

export function flushAuxPersist(): void {
  if (!persistOn || !persistReady) return
  if (saveTimer !== undefined) {
    window.clearTimeout(saveTimer)
    saveTimer = undefined
  }
  void window.api.auxLayout.set({ windows: [...live.values()] } satisfies AuxLayout)
}

function scheduleSave(): void {
  if (!persistOn || !persistReady) return
  if (saveTimer !== undefined) window.clearTimeout(saveTimer)
  saveTimer = window.setTimeout(() => {
    saveTimer = undefined
    void window.api.auxLayout.set({ windows: [...live.values()] } satisfies AuxLayout)
  }, 400)
}

let geoListenStarted = false

/** Геометрия откреплённых окон (в т.ч. после магнита в другом webview). */
export function listenAuxGeoEvents(): () => void {
  if (geoListenStarted) return () => {}
  geoListenStarted = true
  let stop: (() => void) | undefined
  void listen<{ label: string; x: number; y: number; w: number; h: number }>('serein-aux-geo', (e) => {
    const p = e.payload
    if (!p?.label) return
    updateAuxGeometry(p.label, { x: p.x, y: p.y, w: p.w, h: p.h })
  }).then((u) => {
    stop = u
  })
  return () => {
    stop?.()
  }
}
