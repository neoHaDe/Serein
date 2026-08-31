import { describe, it, expect } from 'vitest'
import type { ServerConfig } from '../shared/types'
import {
  collectTags,
  filterServers,
  isEmptyQuery,
  matchesQuery,
  normalizeTags,
  parseServerQuery
} from './serverFilter'

function srv(p: Partial<ServerConfig>): ServerConfig {
  return {
    id: p.id ?? 'id',
    name: p.name ?? 'сервер',
    host: p.host ?? '10.0.0.1',
    port: 22,
    username: p.username ?? 'root',
    authType: 'password',
    ...p
  } as ServerConfig
}

describe('разбор строки поиска', () => {
  it('пустая строка — пустой фильтр', () => {
    expect(isEmptyQuery(parseServerQuery('   '))).toBe(true)
  })

  it('разделяет ключи и свободный текст', () => {
    const q = parseServerQuery('tag:web env:prod fav база данных')
    expect(q.tags).toEqual(['web'])
    expect(q.envs).toEqual(['prod'])
    expect(q.favoriteOnly).toBe(true)
    expect(q.text).toBe('база данных')
  })

  it('ключ без значения не сужает выдачу', () => {
    // Пользователь ещё печатает «tag:». Спрятать в этот момент все серверы — худшее,
    // что может сделать фильтр: список моргает пустотой на каждом вводе.
    const q = parseServerQuery('tag:')
    expect(q.tags).toEqual([])
    expect(isEmptyQuery(q)).toBe(true)
  })

  it('неизвестная среда игнорируется, а не даёт пустой список', () => {
    expect(parseServerQuery('env:боевой').envs).toEqual([])
  })

  it('регистр ключей не важен', () => {
    const q = parseServerQuery('TAG:Web ENV:PROD FAV')
    expect(q.tags).toEqual(['web'])
    expect(q.envs).toEqual(['prod'])
    expect(q.favoriteOnly).toBe(true)
  })
})

describe('отбор серверов', () => {
  const web = srv({ id: 'a', name: 'веб', tags: ['web', 'nginx'], env: 'prod', favorite: true })
  const db = srv({ id: 'b', name: 'база', host: 'db.local', tags: ['db'], env: 'stage' })
  const bare = srv({ id: 'c', name: 'без меток', host: '192.168.1.5' })
  const all = [web, db, bare]

  it('несколько тегов — сервер должен нести все', () => {
    expect(matchesQuery(web, parseServerQuery('tag:web tag:nginx'))).toBe(true)
    expect(matchesQuery(web, parseServerQuery('tag:web tag:db'))).toBe(false)
  })

  it('несколько сред — подходит любая', () => {
    expect(filterServers(all, 'env:prod env:stage').map((s) => s.id)).toEqual(['a', 'b'])
  })

  it('сервер без среды не попадает под фильтр по среде', () => {
    expect(filterServers(all, 'env:prod').map((s) => s.id)).toEqual(['a'])
  })

  it('избранное отбирается отдельно от текста', () => {
    expect(filterServers(all, 'fav').map((s) => s.id)).toEqual(['a'])
    expect(filterServers(all, 'fav база')).toEqual([])
  })

  it('свободный текст ищет и по хосту', () => {
    expect(filterServers(all, 'db.local').map((s) => s.id)).toEqual(['b'])
  })

  it('пустой фильтр возвращает исходный список без копирования', () => {
    expect(filterServers(all, '')).toBe(all)
  })

  it('условия складываются, а не заменяют друг друга', () => {
    expect(filterServers(all, 'tag:web env:prod fav').map((s) => s.id)).toEqual(['a'])
    expect(filterServers(all, 'tag:web env:stage')).toEqual([])
  })
})

describe('нормализация тегов', () => {
  it('убирает пробелы, повторы и регистр', () => {
    // Иначе «Web», «web » и «web» станут тремя тегами, и фильтр по одному из них
    // будет молча терять серверы.
    expect(normalizeTags(' Web , web,  NGINX , ,web ')).toEqual(['web', 'nginx'])
  })

  it('принимает и массив', () => {
    expect(normalizeTags(['A', 'a', 'b'])).toEqual(['a', 'b'])
  })

  it('собирает все теги списка по алфавиту', () => {
    const list = [srv({ id: '1', tags: ['web'] }), srv({ id: '2', tags: ['db', 'web'] })]
    expect(collectTags(list)).toEqual(['db', 'web'])
  })
})
