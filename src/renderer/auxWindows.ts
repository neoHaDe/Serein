import { invoke } from '@tauri-apps/api/core'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'

export async function openAuxWindow(opts: {
  label: string
  query: Record<string, string>
  title: string
  width: number
  height: number
  minWidth?: number
  minHeight?: number
}): Promise<void> {
  const existing = await WebviewWindow.getByLabel(opts.label)
  if (existing) {
    await invoke('windows_raise_group', { focused: opts.label }).catch(() => {})
    await existing.setFocus()
    return
  }
  let auxInTaskbar = false
  try {
    auxInTaskbar = !!(await window.api.settings.get()).auxInTaskbar
  } catch {
    /* */
  }
  const q = new URLSearchParams(opts.query)
  const url = new URL(window.location.href)
  url.search = q.toString()
  url.hash = ''
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
    skipTaskbar: !auxInTaskbar,
    parent: auxInTaskbar ? undefined : 'main'
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
  await invoke('windows_raise_group', { focused: opts.label }).catch(() => {})
}

export function sanitizeWindowLabel(s: string): string {
  return s.replace(/[^a-zA-Z0-9_:-]/g, '').slice(0, 96)
}
