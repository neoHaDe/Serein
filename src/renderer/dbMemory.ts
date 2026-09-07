/**
 * Память панели баз данных между переключениями вкладки.
 *
 * Панель размонтируется, когда человек уходит на другой инструмент, и раньше вместе с
 * ней рвалось соединение: вернувшись, приходилось заново вводить адрес, пароль и запрос.
 * При этом само соединение на стороне сервера живёт своей жизнью — оно привязано к
 * SSH-сессии, а не к тому, открыта ли панель. Поэтому помним, что открыто, и при
 * возвращении показываем то же, что было.
 *
 * Пароль здесь **не хранится** намеренно. Пока соединение живо, он не нужен вовсе, а
 * держать его в памяти дольше, чем открыта форма, — плата без выгоды.
 */

import type { QueryResult } from './dbQuery'

export interface DbForm {
  kind: string
  host: string
  port: string
  user: string
  database: string
}

export interface DbMemory {
  /** Идентификатор живого соединения на стороне приложения. */
  connectionId: string
  /** Что показывать в шапке рядом с заголовком. */
  info: { kind: string; host: string; port: number }
  form: DbForm
  /** Текст запроса и последний результат — чтобы вернуться к тому же экрану. */
  text: string
  result: QueryResult | null
}

const store = new Map<string, DbMemory>()

export function remember(sessionId: string, m: DbMemory): void {
  store.set(sessionId, m)
}

export function recall(sessionId: string): DbMemory | null {
  return store.get(sessionId) ?? null
}

export function forget(sessionId: string): void {
  store.delete(sessionId)
}

/**
 * Обновляет часть запомненного, если запоминать уже есть что.
 *
 * Отдельная функция, потому что текст запроса меняется на каждой букве, а соединение —
 * раз за сеанс: перезаписывать всё целиком означало бы каждый раз собирать объект,
 * рискуя затереть то, чего в этот момент нет под рукой.
 */
export function update(sessionId: string, patch: Partial<DbMemory>): void {
  const cur = store.get(sessionId)
  if (!cur) return
  store.set(sessionId, { ...cur, ...patch })
}

/**
 * Ответ приложения о том, что соединения больше нет.
 *
 * Сессия могла закрыться, пока панель была на другой вкладке: канал живёт внутри неё и
 * умирает вместе с ней. Тогда запомненное соединение — мусор, и держаться за него значит
 * показывать человеку таблицу, за которой ничего нет.
 */
export function isGone(error: string): boolean {
  return /соединение с базой закрыто/i.test(error)
}
