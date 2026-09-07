/**
 * Как показать долю (процессор, память) у процесса, когда её может не быть вовсе.
 *
 * Вынесено из панели и покрыто тестами по той же причине, что и правила запросов к базе:
 * цена ошибки здесь не косметическая. У BusyBox в `ps` нет колонки загрузки процессора -
 * не «ноль», а нечего сообщать. Нарисованный ноль читается как «процесс простаивает», и
 * по такому экрану человек делает выводы о живом сервере.
 */

export interface MetricView {
  /** Что писать в ячейке. */
  text: string
  /** Ширина полоски в процентах; `null` - полоски нет. */
  barPct: number | null
  /** Подсказка при наведении, если значение отсутствует. */
  title?: string
}

const ABSENT: MetricView = {
  text: '—',
  barPct: null,
  title: 'Система не сообщает это значение'
}

export function metricView(value: number | null | undefined): MetricView {
  if (value === null || value === undefined || Number.isNaN(value)) return ABSENT
  return {
    text: value.toFixed(1),
    // Полоска шире ячейки бессмысленна, а отрицательная ломает вёрстку.
    barPct: Math.max(0, Math.min(100, value))
  }
}
