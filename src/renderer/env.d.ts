/// <reference types="vite/client" />
import type { Api } from '../api'

declare global {
  interface Window {
    api: Api
  }
}

export {}
