import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import { DEFAULT_SETTINGS, type AppSettings, type PolicyStatus } from '../shared/types'

interface SettingsCtx {
  settings: AppSettings
  update: (patch: Partial<AppSettings>) => void
  /** Настройки, заданные политикой администратора: менять их нельзя. */
  locked: ReadonlySet<string>
  policy: PolicyStatus | null
}

const NO_LOCKS: ReadonlySet<string> = new Set()

const Ctx = createContext<SettingsCtx>({ settings: DEFAULT_SETTINGS, update: () => {}, locked: NO_LOCKS, policy: null })

export function useSettings(): SettingsCtx {
  return useContext(Ctx)
}

export function SettingsProvider({ children }: { children: ReactNode }): JSX.Element {
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS)
  const [policy, setPolicy] = useState<PolicyStatus | null>(null)
  const locked = policy ? new Set(policy.locked) : NO_LOCKS
  // Накопленный патч и таймер - чтобы не писать settings.json на каждый тик ползунка.
  const pending = useRef<Partial<AppSettings>>({})
  const timer = useRef<ReturnType<typeof setTimeout>>()

  useEffect(() => {
    window.api.settings.get().then(setSettings)
    window.api.policy
      .status()
      .then(setPolicy)
      .catch(() => setPolicy(null))
    return () => {
      // Досрочно сбросить отложенную запись при размонтировании.
      if (timer.current) {
        clearTimeout(timer.current)
        if (Object.keys(pending.current).length) window.api.settings.set(pending.current)
      }
    }
  }, [])

  const update = (patch: Partial<AppSettings>): void => {
    // Заданное политикой не меняется и в окне: бэкенд всё равно отбросит такую правку.
    for (const key of Object.keys(patch)) if (locked.has(key)) delete (patch as Record<string, unknown>)[key]
    if (Object.keys(patch).length === 0) return
    setSettings((prev) => ({ ...prev, ...patch })) // UI реагирует мгновенно
    pending.current = { ...pending.current, ...patch }
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(() => {
      const toSave = pending.current
      pending.current = {}
      window.api.settings.set(toSave)
    }, 300)
  }

  return <Ctx.Provider value={{ settings, update, locked, policy }}>{children}</Ctx.Provider>
}
