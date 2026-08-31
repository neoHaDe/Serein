import { getCurrentWindow } from '@tauri-apps/api/window'

/**
 * Передача владения сессией между окнами.
 *
 * Раньше здесь лежал обычный `Set` — и это было ошибкой уровня архитектуры: модульное
 * состояние у каждого webview своё. Главное окно помечало сессию «откреплённой», а
 * откреплённое об этом не знало вовсе, потому что у него был свой пустой набор. Отсюда
 * баг 1.2.6: окно закрывало сессию, которая ему уже не принадлежала.
 *
 * Теперь владение — один факт на стороне Rust (`ownership.rs`), а закрыть сессию может
 * только окно-владелец. Здесь остались две тонкие обёртки: кому передаём и как называется
 * текущее окно.
 */

/** Метка окна, в котором выполняется этот код. */
export function currentWindowLabel(): string {
  try {
    return getCurrentWindow().label
  } catch {
    // Не в Tauri (тесты, обычный браузер) — метка всё равно ни с чем не совпадёт.
    return 'unknown'
  }
}

/** Отдать сессию окну с указанной меткой. */
export async function handOverSession(sessionId: string, windowLabel: string): Promise<void> {
  if (!sessionId) return
  await window.api.session.claim(sessionId, windowLabel)
}

/** Забрать сессию себе. */
export async function takeOverSession(sessionId: string): Promise<void> {
  await handOverSession(sessionId, currentWindowLabel())
}
