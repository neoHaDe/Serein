import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import { DEFAULT_SETTINGS, type AppSettings } from '../shared/types'

interface SettingsCtx {
  settings: AppSettings
  update: (patch: Partial<AppSettings>) => void
}

const Ctx = createContext<SettingsCtx>({ settings: DEFAULT_SETTINGS, update: () => {} })

export function useSettings(): SettingsCtx {
  return useContext(Ctx)
}

export function SettingsProvider({ children }: { children: ReactNode }): JSX.Element {
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS)
  // Накопленный патч и таймер — чтобы не писать settings.json на каждый тик ползунка.
  const pending = useRef<Partial<AppSettings>>({})
  const timer = useRef<ReturnType<typeof setTimeout>>()

  useEffect(() => {
    window.api.settings.get().then(setSettings)
    return () => {
      // Досрочно сбросить отложенную запись при размонтировании.
      if (timer.current) {
        clearTimeout(timer.current)
        if (Object.keys(pending.current).length) window.api.settings.set(pending.current)
      }
    }
  }, [])

  const update = (patch: Partial<AppSettings>): void => {
    setSettings((prev) => ({ ...prev, ...patch })) // UI реагирует мгновенно
    pending.current = { ...pending.current, ...patch }
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(() => {
      const toSave = pending.current
      pending.current = {}
      window.api.settings.set(toSave)
    }, 300)
  }

  return <Ctx.Provider value={{ settings, update }}>{children}</Ctx.Provider>
}
