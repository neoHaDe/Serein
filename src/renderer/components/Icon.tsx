/**
 * Единый набор векторных иконок (CSP-safe, без внешних шрифтов/спрайтов).
 * Рисуются текущим цветом (currentColor), линиями в стиле Lucide.
 * Заменяют эмодзи в «хроме» интерфейса ради чёткого профессионального вида.
 */
import type { CSSProperties } from 'react'

export type IconName =
  | 'logo'
  | 'server'
  | 'terminal'
  | 'desktop'
  | 'editor'
  | 'broadcast'
  | 'split-h'
  | 'split-v'
  | 'snippets'
  | 'log'
  | 'log-on'
  | 'monitor'
  | 'docker'
  | 'tunnel'
  | 'folder'
  | 'folder-open'
  | 'file'
  | 'key'
  | 'settings'
  | 'import'
  | 'plus'
  | 'close'
  | 'play'
  | 'stop'
  | 'restart'
  | 'refresh'
  | 'edit'
  | 'external'
  | 'trash'
  | 'shell'
  | 'logs'
  | 'back'
  | 'arrow-up'
  | 'chevron-up'
  | 'chevron-down'
  | 'bolt'
  | 'check'
  | 'up-dir'

interface Props {
  name: IconName
  size?: number
  className?: string
  style?: CSSProperties
  title?: string
}

/** SVG-пути (внутренности <svg>) для каждой иконки. viewBox 0 0 24 24, stroke=currentColor. */
const PATHS: Record<IconName, JSX.Element> = {
  logo: (
    <>
      <path d="M4 17l6-5-6-5" />
      <path d="M12 19h8" />
    </>
  ),
  server: (
    <>
      <rect x="3" y="4" width="18" height="7" rx="1.5" />
      <rect x="3" y="13" width="18" height="7" rx="1.5" />
      <path d="M7 7.5h.01M7 16.5h.01" />
    </>
  ),
  terminal: (
    <>
      <path d="M5 7l4 4-4 4" />
      <path d="M12 16h7" />
    </>
  ),
  desktop: (
    <>
      <rect x="3" y="4" width="18" height="12" rx="1.5" />
      <path d="M8 20h8M12 16v4" />
    </>
  ),
  editor: (
    <>
      <path d="M14 3v5h5" />
      <path d="M19 8.5V20a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h8z" />
      <path d="M8.5 13l2 2 4-4" />
    </>
  ),
  broadcast: (
    <>
      <circle cx="12" cy="12" r="2" />
      <path d="M7.8 7.8a6 6 0 0 0 0 8.4M16.2 16.2a6 6 0 0 0 0-8.4" />
      <path d="M5 5a10 10 0 0 0 0 14M19 19a10 10 0 0 0 0-14" />
    </>
  ),
  'split-h': (
    <>
      <rect x="3" y="4" width="18" height="16" rx="1.5" />
      <path d="M12 4v16" />
    </>
  ),
  'split-v': (
    <>
      <rect x="3" y="4" width="18" height="16" rx="1.5" />
      <path d="M3 12h18" />
    </>
  ),
  snippets: (
    <>
      <rect x="8" y="3" width="8" height="4" rx="1" />
      <path d="M9 5H6a1 1 0 0 0-1 1v13a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1h-3" />
      <path d="M8.5 11h7M8.5 15h4" />
    </>
  ),
  log: (
    <>
      <path d="M14 3v5h5" />
      <path d="M19 8.5V20a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h8z" />
      <path d="M8.5 12h7M8.5 16h5" />
    </>
  ),
  'log-on': (
    <>
      <path d="M14 3v5h5" />
      <path d="M19 8.5V20a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h8z" />
      <circle cx="12" cy="14" r="2.2" fill="currentColor" stroke="none" />
    </>
  ),
  monitor: (
    <>
      <path d="M3 12h4l2 5 4-12 2 7h6" />
    </>
  ),
  docker: (
    <>
      <rect x="3" y="10" width="4" height="4" rx="0.5" />
      <rect x="8" y="10" width="4" height="4" rx="0.5" />
      <rect x="13" y="10" width="4" height="4" rx="0.5" />
      <rect x="8" y="5" width="4" height="4" rx="0.5" />
      <path d="M3 14c0 3 2.5 5 7 5 6 0 9-3.5 9.5-7 0 0 1.5.2 1.5-1.5" />
    </>
  ),
  tunnel: (
    <>
      <path d="M4 8h11a4 4 0 0 1 0 8H9" />
      <path d="M7 13l-3 3 3 3" />
    </>
  ),
  folder: (
    <>
      <path d="M3 7a1 1 0 0 1 1-1h5l2 2h8a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z" />
    </>
  ),
  'folder-open': (
    <>
      <path d="M3 7a1 1 0 0 1 1-1h5l2 2h8a1 1 0 0 1 1 1v1H5l-2 8" />
      <path d="M3 18l2-7a1 1 0 0 1 1-.8h14.5a1 1 0 0 1 1 1.2L21 18z" />
    </>
  ),
  file: (
    <>
      <path d="M13 3v5h5" />
      <path d="M18 8.5V20a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h6z" />
    </>
  ),
  key: (
    <>
      <circle cx="8" cy="12" r="4" />
      <path d="M11 12h9l-2 2.5M16 12v3" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M19.1 4.9L17 7M7 17l-2.1 2.1" />
    </>
  ),
  import: (
    <>
      <path d="M12 3v11" />
      <path d="M8 11l4 4 4-4" />
      <path d="M5 19h14" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  close: <path d="M6 6l12 12M18 6L6 18" />,
  play: <path d="M7 5l11 7-11 7z" />,
  stop: <rect x="6" y="6" width="12" height="12" rx="1.5" />,
  restart: (
    <>
      <path d="M20 12a8 8 0 1 1-2.3-5.6" />
      <path d="M20 4v4h-4" />
    </>
  ),
  refresh: (
    <>
      <path d="M20 12a8 8 0 1 1-2.3-5.6" />
      <path d="M20 4v4h-4" />
    </>
  ),
  edit: (
    <>
      <path d="M4 20h4l10-10-4-4L4 16z" />
      <path d="M13.5 6.5l4 4" />
    </>
  ),
  external: (
    <>
      <path d="M14 4h6v6" />
      <path d="M20 4l-8 8" />
      <path d="M18 13v5a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" />
    </>
  ),
  trash: (
    <>
      <path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2" />
      <path d="M6 7l1 13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-13" />
    </>
  ),
  shell: (
    <>
      <path d="M5 7l4 4-4 4" />
      <path d="M12 16h7" />
    </>
  ),
  logs: (
    <>
      <path d="M5 6h14M5 10h14M5 14h10M5 18h7" />
    </>
  ),
  back: <path d="M15 6l-6 6 6 6" />,
  'arrow-up': <path d="M12 19V5M6 11l6-6 6 6" />,
  'chevron-up': <path d="M6 15l6-6 6 6" />,
  'chevron-down': <path d="M6 9l6 6 6-6" />,
  bolt: <path d="M13 2L4 14h7l-1 8 9-12h-7z" />,
  check: <path d="M5 12l5 5L20 7" />,
  'up-dir': (
    <>
      <path d="M12 19V8M7 12l5-5 5 5" />
      <path d="M5 5h14" />
    </>
  )
}

export function Icon({ name, size = 16, className, style, title }: Props): JSX.Element {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      style={style}
      aria-hidden={title ? undefined : true}
      role={title ? 'img' : undefined}
      focusable="false"
    >
      {title && <title>{title}</title>}
      {PATHS[name]}
    </svg>
  )
}
