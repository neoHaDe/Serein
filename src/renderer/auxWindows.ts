import { invoke } from '@tauri-apps/api/core'
import { PhysicalPosition, PhysicalSize } from '@tauri-apps/api/dpi'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import type { SavedAuxWindow } from '../shared/types'
import { registerAuxWindow, unregisterAuxWindow, updateAuxGeometry } from './auxLayout'

export type AuxPersistMeta = Pick<SavedAuxWindow, 'kind' | 'serverId' | 'containerId' | 'name'>

async function applyInnerGeo(
  w: WebviewWindow,
  x: number,
  y: number,
  width: number,
  height: number
): Promise<void> {
  try {
    await w.setSize(new PhysicalSize(Math.round(width), Math.round(height)))
    const outer = await w.outerPosition()
    const inner = await w.innerPosition()
    const padX = inner.x - outer.x
    const padY = inner.y - outer.y
    await w.setPosition(new PhysicalPosition(Math.round(x - padX), Math.round(y - padY)))
  } catch {
    /* */
  }
}

async function snapshotGeo(w: WebviewWindow, label: string): Promise<void> {
  try {
    const inner = await w.innerPosition()
    const size = await w.innerSize()
    updateAuxGeometry(label, { x: inner.x, y: inner.y, w: size.width, h: size.height })
  } catch {
    /* */
  }
}

function trackAux(
  w: WebviewWindow,
  label: string,
  persist: AuxPersistMeta,
  x: number,
  y: number,
  width: number,
  height: number
): void {
  registerAuxWindow(label, {
    kind: persist.kind,
    serverId: persist.serverId,
    containerId: persist.containerId,
    name: persist.name,
    x,
    y,
    w: width,
    h: height
  })
  void snapshotGeo(w, label)
  void w.onMoved(() => {
    void snapshotGeo(w, label)
  })
  void w.onResized(() => {
    void snapshotGeo(w, label)
  })
  void w.once('tauri://destroyed', () => {
    unregisterAuxWindow(label)
  })
}

export async function openAuxWindow(opts: {
  label: string
  query: Record<string, string>
  title: string
  width: number
  height: number
  minWidth?: number
  minHeight?: number
  x?: number
  y?: number
  persist?: AuxPersistMeta
}): Promise<void> {
  const q = new URLSearchParams(opts.query)
  if (import.meta.env.DEV) q.set('_cb', String(Date.now()))
  const url = new URL(window.location.href)
  url.search = q.toString()
  url.hash = ''
  const existing = await WebviewWindow.getByLabel(opts.label)
  if (existing) {
    try {
      await existing.close()
    } catch {
      /* */
    }
    await new Promise((r) => window.setTimeout(r, 80))
  }
  let auxInTaskbar = false
  try {
    auxInTaskbar = !!(await window.api.settings.get()).auxInTaskbar
  } catch {
    /* */
  }
  const w = new WebviewWindow(opts.label, {
    url: url.href,
    title: opts.title,
    width: Math.round(opts.width),
    height: Math.round(opts.height),
    minWidth: opts.minWidth ?? 420,
    minHeight: opts.minHeight ?? 280,
    resizable: true,
    focus: true,
    decorations: false,
    shadow: false,
    // На Windows встроенный перехват перетаскивания глушит HTML5 drag внутри страницы,
    // а события ОС мы не используем — поэтому выключаем и здесь, как в главном окне.
    dragDropEnabled: false,
    skipTaskbar: !auxInTaskbar
  })
  await new Promise<void>((resolve, reject) => {
    const t = window.setTimeout(() => resolve(), 4000)
    void w.once('tauri://created', () => {
      window.clearTimeout(t)
      resolve()
    })
    void w.once('tauri://error', (e) => {
      window.clearTimeout(t)
      reject(new Error(String(e.payload ?? e)))
    })
  })
  if (opts.x != null && opts.y != null) {
    await applyInnerGeo(w, opts.x, opts.y, opts.width, opts.height)
  }
  if (opts.persist) {
    trackAux(w, opts.label, opts.persist, opts.x ?? 0, opts.y ?? 0, opts.width, opts.height)
  }
  await invoke('windows_raise_group', { focused: opts.label }).catch(() => {})
}

export function sanitizeWindowLabel(s: string): string {
  return s.replace(/[^a-zA-Z0-9_:-]/g, '').slice(0, 96)
}
