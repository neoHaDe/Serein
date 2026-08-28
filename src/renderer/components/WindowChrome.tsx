import { useEffect, useState, type ReactNode } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Icon } from './Icon'
import { notifyWindowMinimize } from '../windowSnap'

async function minimizeApp(w: ReturnType<typeof getCurrentWindow>): Promise<void> {
  await notifyWindowMinimize()
  await w.minimize()
}

export function WindowSysButtons(): JSX.Element {
  const [maxed, setMaxed] = useState(false)

  useEffect(() => {
    const w = getCurrentWindow()
    void w.isMaximized().then(setMaxed)
    let off: (() => void) | undefined
    void w.onResized(() => {
      void w.isMaximized().then(setMaxed)
    }).then((u) => {
      off = u
    })
    return () => off?.()
  }, [])

  const w = getCurrentWindow()
  return (
    <div className="win-sys">
      <button type="button" className="win-sys-btn" title="Свернуть" onClick={() => void minimizeApp(w)}>
        <Icon name="win-min" size={12} />
      </button>
      <button
        type="button"
        className="win-sys-btn"
        title={maxed ? "Восстановить" : "Развернуть"}
        onClick={() => void w.toggleMaximize()}
      >
        <Icon name={maxed ? "win-restore" : "win-max"} size={12} />
      </button>
      <button type="button" className="win-sys-btn is-close" title="Закрыть" onClick={() => void w.close()}>
        <Icon name="close" size={13} />
      </button>
    </div>
  )
}

export function AuxDrag({ children }: { children?: ReactNode }): JSX.Element {
  return (
    <div
      className="aux-drag"
      data-tauri-drag-region
      onDoubleClick={() => {
        void getCurrentWindow().toggleMaximize()
      }}
    >
      {children}
    </div>
  )
}

export function markAuxWindow(): void {
  document.documentElement.classList.add('aux-win', 'frameless')
}

export function AppChrome({ title, actions }: { title: string; actions?: ReactNode }): JSX.Element {
  return (
    <div className="app-chrome">
      <AuxDrag>
        <span className="app-chrome-title">{title}</span>
      </AuxDrag>
      {actions ? <div className="app-chrome-actions">{actions}</div> : null}
      <WindowSysButtons />
    </div>
  )
}
