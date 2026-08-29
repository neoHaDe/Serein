import { invoke } from '@tauri-apps/api/core'

type Platform = 'windows' | 'linux' | 'other'

let cached: Promise<Platform> | null = null

export function appPlatform(): Promise<Platform> {
  if (!cached) {
    cached = invoke<string>('app_platform').then((p) => {
      if (p === 'windows' || p === 'linux') return p
      return 'other'
    })
  }
  return cached
}

export function isWindowsPlatform(): Promise<boolean> {
  return appPlatform().then((p) => p === 'windows')
}
