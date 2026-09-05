/**
 * Правила вокруг запроса к базе: что показать после выполнения и о чём предупредить до.
 *
 * Вынесено из панели и покрыто тестами, потому что цена ошибки здесь не косметическая.
 * Запрос выполняется на живой базе рядом с рабочим сервером, и `DELETE` без условия
 * отличается от `DELETE ... WHERE` одним словом.
 */

export interface QueryResult {
  columns: string[]
  rows: Record<string, unknown>[]
  affected: number
  ms: number
}

/** Человеческий итог: сколько строк и за сколько. Показывается под таблицей. */
export function summarize(r: QueryResult): string {
  const time = `${r.ms} мс`
  if (r.rows.length > 0) return `${r.rows.length} ${plural(r.rows.length, 'строка', 'строки', 'строк')} · ${time}`
  if (r.affected > 0) {
    return `изменено ${r.affected} ${plural(r.affected, 'строка', 'строки', 'строк')} · ${time}`
  }
  return `выполнено · ${time}`
}

function plural(n: number, one: string, few: string, many: string): string {
  const mod100 = n % 100
  if (mod100 >= 11 && mod100 <= 14) return many
  switch (n % 10) {
    case 1:
      return one
    case 2:
    case 3:
    case 4:
      return few
    default:
      return many
  }
}

/**
 * Причина, по которой запрос стоит подтвердить, или `null`, если подтверждать нечего.
 *
 * Смотрим не на «опасные слова» вообще, а на то, что необратимо и не ограничено условием.
 * `DELETE ... WHERE` — обычная работа, а `DELETE` без него вычищает таблицу целиком, и
 * узнать об этом хочется до выполнения, а не из отчёта «изменено 400000 строк».
 */
export function needsConfirm(sql: string): string | null {
  const text = strip(sql)
  if (!text) return null

  if (/^\s*drop\s+(table|database|schema|index)\b/i.test(text)) {
    return 'Запрос удаляет объект базы целиком. Это необратимо.'
  }
  if (/^\s*truncate\b/i.test(text)) {
    return 'TRUNCATE очищает таблицу целиком и обычно не откатывается.'
  }
  if (/^\s*delete\s+from\b/i.test(text) && !/\bwhere\b/i.test(text)) {
    return 'DELETE без WHERE удалит все строки таблицы.'
  }
  if (/^\s*update\b/i.test(text) && !/\bwhere\b/i.test(text)) {
    return 'UPDATE без WHERE изменит все строки таблицы.'
  }
  // Redis: команды, стирающие базу целиком, тоже без пути назад.
  if (/^\s*(flushall|flushdb)\b/i.test(text)) {
    return 'Команда стирает базу целиком.'
  }
  return null
}

/**
 * Убирает комментарии и строковые литералы перед разбором.
 *
 * Без этого `DELETE FROM t -- WHERE потом допишу` сойдёт за безопасный запрос: слово
 * `where` в комментарии есть, а условия нет.
 */
function strip(sql: string): string {
  return sql
    .replace(/'(?:[^']|'')*'/g, "''")
    .replace(/--[^\n]*/g, ' ')
    .replace(/\/\*[\s\S]*?\*\//g, ' ')
}

/** Значение ячейки для показа: NULL надо отличать от пустой строки. */
export function cellText(v: unknown): string {
  if (v === null || v === undefined) return 'NULL'
  if (typeof v === 'string') return v
  return JSON.stringify(v)
}

export function isNull(v: unknown): boolean {
  return v === null || v === undefined
}
