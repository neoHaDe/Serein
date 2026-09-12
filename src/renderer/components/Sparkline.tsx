/**
 * Маленький график за последний час.
 *
 * Своим SVG, без библиотеки графиков: линия, две пунктирные черты порогов и подпись. Ради
 * этого тянуть в поставку отдельную библиотеку - лишние мегабайты и лишняя зависимость.
 */

/** Разрыв между точками, после которого линию рвём: данных за отрезок нет. */
const GAP_MS = 90 * 1000

const W = 240
const H = 44

export interface SparkPoint {
  t: number
  v: number
}

interface Props {
  title: string
  /** Текущее значение словами - то, что человек прочтёт первым. */
  value: string
  points: SparkPoint[]
  /** Верх шкалы. Для процентов - 100, иначе берётся по данным. */
  max?: number
  warn?: number
  bad?: number
  spanMs: number
  now: number
}

export function Sparkline({ title, value, points, max, warn, bad, spanMs, now }: Props): JSX.Element {
  const shown = points.filter((p) => p.t >= now - spanMs)
  const peak = shown.reduce((acc, p) => Math.max(acc, p.v), 0)
  const top = max ?? Math.max(peak * 1.15, 1)
  const x = (t: number): number => W - ((now - t) / spanMs) * W
  const y = (v: number): number => H - (Math.min(Math.max(v, 0), top) / top) * H

  // Разрывы в данных рисуем разрывами, а не прямой через пустоту: иначе график врёт, что
  // всё это время сервер был под наблюдением.
  const segments: string[] = []
  let current: string[] = []
  let prev: number | undefined
  for (const p of shown) {
    if (prev !== undefined && p.t - prev > GAP_MS && current.length) {
      segments.push(current.join(' '))
      current = []
    }
    current.push(`${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`)
    prev = p.t
  }
  if (current.length) segments.push(current.join(' '))

  return (
    <div className="spark">
      <div className="spark-head">
        <span className="spark-title">{title}</span>
        <span className="spark-value mono">{value}</span>
      </div>
      <svg
        className="spark-svg"
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        role="img"
        aria-label={`${title} за час: сейчас ${value}`}
      >
        {warn !== undefined && warn < top && (
          <line className="spark-warn" x1={0} x2={W} y1={y(warn)} y2={y(warn)} />
        )}
        {bad !== undefined && bad < top && (
          <line className="spark-bad" x1={0} x2={W} y1={y(bad)} y2={y(bad)} />
        )}
        {segments.map((s, i) => (
          <polyline key={i} className="spark-line" points={s} />
        ))}
      </svg>
      <div className="spark-axis">
        <span>час назад</span>
        {shown.length === 0 && <span>данные копятся</span>}
        <span>сейчас</span>
      </div>
    </div>
  )
}
