/**
 * Текст ошибки для показа пользователю.
 *
 * Команды Rust возвращают `Result<_, String>`, и Tauri бросает **саму строку**, а не `Error`.
 * Поэтому привычное `(e as Error).message` даёт `undefined`, и человек видит
 * «Ошибка импорта: undefined» вместо «Файл ~/.ssh/config не найден».
 */
export function errText(e: unknown): string {
  if (typeof e === 'string') return e
  if (e instanceof Error) return e.message
  if (e && typeof e === 'object') {
    const msg = (e as { message?: unknown }).message
    if (typeof msg === 'string') return msg
    try {
      return JSON.stringify(e)
    } catch {
      /* по кругу — отдадим общий текст ниже */
    }
  }
  return String(e ?? 'Неизвестная ошибка')
}
