import { useEffect } from 'react'
import { emit } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { applyUiTheme } from '../themes'
import { useWindowSnap } from '../windowSnap'
import { markAuxWindow } from './WindowChrome'
import { SftpPanel } from './SftpPanel'

export function SftpWindow(): JSX.Element {
  const sessionId = new URLSearchParams(window.location.search).get('sessionId') ?? ''
  useWindowSnap()

  useEffect(() => {
    markAuxWindow()
    void window.api.settings.get().then((s) => {
      applyUiTheme(s.theme)
      document.documentElement.dataset.density = s.density ?? 'comfortable'
    })
  }, [])

  return (
    <SftpPanel
      sessionId={sessionId}
      width={480}
      closing={false}
      detached
      onClose={() => {
        void getCurrentWindow().close()
      }}
      onOpenInEditor={(remotePath) => {
        void emit('serein-open-editor', { sessionId, remotePath })
      }}
    />
  )
}
