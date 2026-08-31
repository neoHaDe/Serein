import type { ServerConfig, ServerEnv } from '../shared/types'

/**
 * Разбор и применение фильтра списка серверов.
 *
 * Когда серверов десятки, одного поиска по имени мало: нужно «покажи прод», «покажи всё
 * с тегом web», «покажи избранное». Поэтому строка поиска понимает несколько ключей, а не
 * только свободный текст. Логика вынесена из сайдбара сюда, чтобы её можно было проверить
 * тестами: в разметке такие правила проверяются только глазами и разъезжаются молча.
 */

export const ENVS: ServerEnv[] = ['prod', 'stage', 'dev']

/** Человеческие названия сред для интерфейса. */
export const ENV_LABEL: Record<ServerEnv, string> = {
  prod: 'PROD',
  stage: 'STAGE',
  dev: 'DEV'
}

export interface ServerQuery {
  /** Свободный текст: имя, хост, пользователь, COM-порт. */
  text: string
  /** Теги: сервер должен нести их все — так фильтр сужается предсказуемо. */
  tags: string[]
  /** Среды: подходит любая из перечисленных. */
  envs: ServerEnv[]
  /** Показывать только избранные. */
  favoriteOnly: boolean
}

function isEnv(v: string): v is ServerEnv {
  return (ENVS as string[]).includes(v)
}

/**
 * Разбирает строку поиска.
 *
 * `tag:web env:prod fav база` → теги `web`, среда `prod`, только избранное, текст «база».
 * Ключ без значения (`tag:`) игнорируется: пользователь ещё печатает, и убирать из выдачи
 * всё подряд на полпути — худшее, что можно сделать.
 */
export function parseServerQuery(raw: string): ServerQuery {
  const q: ServerQuery = { text: '', tags: [], envs: [], favoriteOnly: false }
  const words: string[] = []

  for (const token of raw.trim().split(/\s+/).filter(Boolean)) {
    const lower = token.toLowerCase()
    if (lower === 'fav' || lower === 'fav:') {
      q.favoriteOnly = true
    } else if (lower.startsWith('tag:')) {
      const v = lower.slice(4)
      if (v) q.tags.push(v)
    } else if (lower.startsWith('env:')) {
      const v = lower.slice(4)
      if (isEnv(v)) q.envs.push(v)
    } else {
      words.push(token)
    }
  }

  q.text = words.join(' ').toLowerCase()
  return q
}

/** Пустой ли фильтр — то есть показывать ли всё. */
export function isEmptyQuery(q: ServerQuery): boolean {
  return !q.text && q.tags.length === 0 && q.envs.length === 0 && !q.favoriteOnly
}

function matchesText(s: ServerConfig, text: string): boolean {
  if (!text) return true
  return (
    s.name.toLowerCase().includes(text) ||
    s.host.toLowerCase().includes(text) ||
    s.username.toLowerCase().includes(text) ||
    (s.serial?.port ?? '').toLowerCase().includes(text)
  )
}

/** Подходит ли сервер под разобранный фильтр. */
export function matchesQuery(s: ServerConfig, q: ServerQuery): boolean {
  if (q.favoriteOnly && !s.favorite) return false
  if (q.envs.length > 0 && (!s.env || !q.envs.includes(s.env))) return false
  if (q.tags.length > 0) {
    const own = (s.tags ?? []).map((t) => t.toLowerCase())
    if (!q.tags.every((t) => own.includes(t))) return false
  }
  return matchesText(s, q.text)
}

/** Отфильтрованный список в исходном порядке. */
export function filterServers(servers: ServerConfig[], raw: string): ServerConfig[] {
  const q = parseServerQuery(raw)
  if (isEmptyQuery(q)) return servers
  return servers.filter((s) => matchesQuery(s, q))
}

/**
 * Приводит теги к хранимому виду: без пробелов по краям, без пустых, без повторов,
 * в нижнем регистре. Иначе «Web», «web » и «web» станут тремя разными тегами, и фильтр
 * по любому из них будет терять серверы.
 */
export function normalizeTags(input: string | string[]): string[] {
  const parts = Array.isArray(input) ? input : input.split(',')
  const out: string[] = []
  for (const p of parts) {
    const t = p.trim().toLowerCase()
    if (t && !out.includes(t)) out.push(t)
  }
  return out
}

/** Все теги, встречающиеся у серверов — для подсказок и быстрых фильтров. */
export function collectTags(servers: ServerConfig[]): string[] {
  const seen = new Set<string>()
  for (const s of servers) for (const t of s.tags ?? []) seen.add(t)
  return [...seen].sort((a, b) => a.localeCompare(b))
}
