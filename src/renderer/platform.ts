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

/** Как приложение установлено: от этого зависит, можно ли обновиться на месте. */
export type InstallKind = 'installer' | 'appimage' | 'package'

let cachedKind: Promise<InstallKind> | null = null

/**
 * Windows — `installer`, Linux из AppImage — `appimage`, Linux из пакета — `package`.
 * Как и с платформой, промис не отклоняется: неизвестность трактуем как пакет,
 * то есть предлагаем скачать вручную вместо установки, которая всё равно не пройдёт.
 */
export function installKind(): Promise<InstallKind> {
  if (!cachedKind) {
    cachedKind = invoke<string>('app_install_kind')
      .then((k): InstallKind =>
        k === 'installer' || k === 'appimage' ? k : 'package'
      )
      .catch((): InstallKind => 'package')
  }
  return cachedKind
}
