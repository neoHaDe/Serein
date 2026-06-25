import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],
  // Точка входа — renderer/index.html (сохранена раскладка исходного TermiNAL).
  root: '.',
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // На Windows `localhost` резолвится в ::1 первым → Tauri висит на «Waiting for
    // frontend dev server». Форсим IPv4. См. insight tauri-dev-ipv4-localhost.
    host: host || '127.0.0.1',
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] }
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true
  }
}))
