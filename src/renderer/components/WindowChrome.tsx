import { useEffect, useState, type ReactNode } from 'react'
import { getCurrentWindow, Window } from '@tauri-apps/api/window'
import { Icon } from './Icon'

async function minimizeApp(w: ReturnType<typeof getCurrentWindow>): Promise<void> {
  if (w.label === 'main') {
    await w.minimize()
    return
  }
  const main = await Window.getByLabel('main')
  if (main) await main.minimize()
  else await w.minimize()
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

export function AppChrome({ title }: { title: string }): JSX.Element {
  return (
    <div className="app-chrome">
      <AuxDrag>
        <span className="app-chrome-title">{title}</span>
      </AuxDrag>
      <WindowSysButtons />
    </div>
  )
}
