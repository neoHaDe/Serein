/**
 * Порты контейнера из `docker ps` для человека.
 *
 * Docker перечисляет каждую привязку отдельно для IPv4 и IPv6 и для каждого протокола:
 * «0.0.0.0:25565->25565/tcp, [::]:25565->25565/tcp, 0.0.0.0:25565->25565/udp, …». Прежний
 * разбор выбрасывал адрес и протокол и оставлял четыре одинаковых «25565 → 25565», а диапазоны
 * вида «80-81->80-81» не узнавал вовсе и показывал сырыми.
 */

/** Одна привязка: внешний порт → порт контейнера, протокол; или только открытый порт. */
function parsePart(part: string): string {
  const s = part.trim()
  const published = /^(?:\[[^\]]*\]|[^:\s]+):(\d+(?:-\d+)?)->(\d+(?:-\d+)?)\/(\w+)$/.exec(s)
  if (published) {
    const [, host, container, proto] = published
    return `${host} → ${container}${proto === 'tcp' ? '' : '/' + proto}`
  }
  const exposed = /^(\d+(?:-\d+)?)\/(\w+)$/.exec(s)
  if (exposed) {
    const [, port, proto] = exposed
    return `${port}${proto === 'tcp' ? '' : '/' + proto}`
  }
  return s
}

export function formatPorts(raw?: string): string {
  if (!raw?.trim()) return '—'
  const seen = new Set<string>()
  const out: string[] = []
  for (const part of raw.split(',')) {
    const text = parsePart(part)
    if (text && !seen.has(text)) {
      seen.add(text)
      out.push(text)
    }
  }
  return out.join(', ')
}
