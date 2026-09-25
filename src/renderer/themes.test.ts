import { describe, expect, it } from 'vitest'
import css from './styles.css?raw'
import { DEFAULT_SETTINGS } from '../shared/types'
import { uiVars } from './themes'

/** Запасные значения из первого блока `:root` - ими рисуется первый кадр. */
function fallbackVars(): Record<string, string> {
  const root = /:root\s*\{([\s\S]*?)\n\}/.exec(css)?.[1] ?? ''
  const out: Record<string, string> = {}
  for (const m of root.matchAll(/(--[\w-]+):\s*([^;]+);/g)) out[m[1]!] = m[2]!.trim()
  return out
}

describe('первый кадр', () => {
  it('рисуется палитрой темы по умолчанию', () => {
    // Запасные цвета в стилях стояли от Tokyo Night уже после того, как темой по
    // умолчанию стала GitHub Dark: окно при запуске мигало чужой палитрой.
    const fallback = fallbackVars()
    for (const [name, value] of Object.entries(uiVars(DEFAULT_SETTINGS.theme))) {
      expect(fallback[name], name).toBe(value)
    }
  })
})
