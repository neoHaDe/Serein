import type { HealthThresholds, Threshold } from '../../shared/types'

/**
 * Редактор порогов здоровья: одна таблица на общие настройки и на карточку сервера.
 *
 * Одна - потому что разъехавшиеся формы порогов в двух местах гарантируют, что однажды
 * «диск» в настройках и «диск» у сервера начнут значить разное.
 */

interface Row {
  key: keyof HealthThresholds
  label: string
  step: number
  hint: string
}

const ROWS: Row[] = [
  { key: 'cpu', label: 'Процессор, %', step: 1, hint: 'причиной становится, если держится выше порога пять минут подряд' },
  { key: 'mem', label: 'Память, %', step: 1, hint: 'сколько занято от всей памяти' },
  { key: 'disk', label: 'Диск, %', step: 1, hint: 'каждый том проверяется отдельно' },
  { key: 'load', label: 'Загрузка на ядро', step: 0.05, hint: '1.0 - работы ровно столько, сколько ядер' }
]

interface Props {
  /** Что задано в этом слое. Незаданное берётся из `base`. */
  value: Partial<HealthThresholds> | undefined
  /** Что действует без этого слоя: умолчания для настроек, общие пороги для сервера. */
  base: HealthThresholds
  onChange: (next: Partial<HealthThresholds>) => void
}

export function ThresholdsEditor({ value, base, onChange }: Props): JSX.Element {
  const set = (key: keyof HealthThresholds, part: keyof Threshold, raw: string): void => {
    const n = Number(raw.replace(',', '.'))
    if (!Number.isFinite(n) || n < 0) return
    const current = { ...base[key], ...value?.[key] }
    onChange({ ...value, [key]: { ...current, [part]: n } })
  }

  return (
    <div className="th-grid">
      <span className="th-head" />
      <span className="th-head">внимание с</span>
      <span className="th-head">плохо с</span>
      {ROWS.map((r) => {
        const t = { ...base[r.key], ...value?.[r.key] }
        return (
          <div key={r.key} className="th-row" style={{ display: 'contents' }}>
            <span>{r.label}</span>
            <input
              type="number"
              min={0}
              step={r.step}
              value={t.warn}
              onChange={(e) => set(r.key, 'warn', e.target.value)}
            />
            <input
              type="number"
              min={0}
              step={r.step}
              value={t.bad}
              onChange={(e) => set(r.key, 'bad', e.target.value)}
            />
            <span className="th-hint">{r.hint}</span>
          </div>
        )
      })}
    </div>
  )
}
