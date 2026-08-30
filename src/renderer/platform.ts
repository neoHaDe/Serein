import { invoke } from '@tauri-apps/api/core'

type Platform = 'windows' | 'linux' | 'other'

let cached: Promise<Platform> | null = null

/**
 * На какой системе мы работаем. Ответ кэшируется — команда возвращает константу.
 *
 * Промис не отклоняется никогда, и это важно: результат ждут в том числе при настройке
 * магнетизма окон, а там всё обёрнуто в общий `try` со смыслом «мы не в Tauri». Одна
 * сорвавшаяся команда отключила бы примагничивание целиком и молча — до перезапуска.
 */
export function appPlatform(): Promise<Platform> {
  if (!cached) {
    cached = invoke<string>('app_platform')
      .then((p): Platform => (p === 'windows' || p === 'linux' ? p : 'other'))
      .catch((): Platform => 'other')
  }
  return cached
}

export function isWindowsPlatform(): Promise<boolean> {
  return appPlatform().then((p) => p === 'windows')
}
