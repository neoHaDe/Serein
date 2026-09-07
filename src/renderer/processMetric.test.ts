import { describe, it, expect } from 'vitest'
import { metricView } from './processMetric'

describe('доля в таблице процессов', () => {
  it('обычное значение показывается с одним знаком и полоской', () => {
    expect(metricView(12.34)).toEqual({ text: '12.3', barPct: 12.34 })
    expect(metricView(0)).toEqual({ text: '0.0', barPct: 0 })
  })

  it('отсутствие значения - прочерк, а не ноль', () => {
    // Главное различие всей затеи: у BusyBox `ps` не сообщает загрузку процессора.
    // Ноль на этом месте читался бы как «процесс простаивает» - это неправда.
    for (const absent of [null, undefined, NaN]) {
      const v = metricView(absent)
      expect(v.text).toBe('—')
      expect(v.barPct).toBeNull()
      expect(v.title).toMatch(/не сообщает/)
    }
  })

  it('ноль и отсутствие показываются по-разному', () => {
    expect(metricView(0).text).not.toBe(metricView(null).text)
  })

  it('полоска не вылезает за ячейку и не уходит в минус', () => {
    // Суммарный процессор у многоядерной машины бывает и больше ста.
    expect(metricView(340).barPct).toBe(100)
    expect(metricView(-1).barPct).toBe(0)
  })
})
