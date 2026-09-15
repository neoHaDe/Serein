import { describe, it, expect } from 'vitest'
import { formatPorts } from './dockerPorts'

describe('порты контейнера', () => {
  it('склеивает IPv4 и IPv6, протокол показывает только не-TCP', () => {
    expect(
      formatPorts('0.0.0.0:25565->25565/tcp, [::]:25565->25565/tcp, 0.0.0.0:25565->25565/udp, [::]:25565->25565/udp')
    ).toBe('25565 → 25565, 25565 → 25565/udp')
  })

  it('понимает диапазоны и открытые без публикации порты', () => {
    expect(formatPorts('0.0.0.0:80-81->80-81/tcp, [::]:80-81->80-81/tcp, 3306/tcp')).toBe('80-81 → 80-81, 3306')
    expect(formatPorts('127.0.0.1:8081->8080/tcp')).toBe('8081 → 8080')
  })

  it('пустое - прочерк, непонятное - как есть', () => {
    expect(formatPorts('')).toBe('—')
    expect(formatPorts(undefined)).toBe('—')
    expect(formatPorts('странное')).toBe('странное')
  })
})
