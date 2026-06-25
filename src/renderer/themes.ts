import type { ITheme } from '@xterm/xterm'

/** Цветовые схемы терминала. Ключ — отображаемое имя. */
export const THEMES: Record<string, ITheme> = {
  'Tokyo Night': {
    background: '#1a1b26', foreground: '#c0caf5', cursor: '#c0caf5', selectionBackground: '#33467c',
    black: '#15161e', red: '#f7768e', green: '#9ece6a', yellow: '#e0af68',
    blue: '#7aa2f7', magenta: '#bb9af7', cyan: '#7dcfff', white: '#a9b1d6',
    brightBlack: '#414868', brightRed: '#f7768e', brightGreen: '#9ece6a', brightYellow: '#e0af68',
    brightBlue: '#7aa2f7', brightMagenta: '#bb9af7', brightCyan: '#7dcfff', brightWhite: '#c0caf5'
  },
  'Tokyo Night Storm': {
    background: '#24283b', foreground: '#c0caf5', cursor: '#c0caf5', selectionBackground: '#364a82',
    black: '#1d202f', red: '#f7768e', green: '#9ece6a', yellow: '#e0af68',
    blue: '#7aa2f7', magenta: '#bb9af7', cyan: '#7dcfff', white: '#a9b1d6',
    brightBlack: '#414868', brightRed: '#f7768e', brightGreen: '#9ece6a', brightYellow: '#e0af68',
    brightBlue: '#7aa2f7', brightMagenta: '#bb9af7', brightCyan: '#7dcfff', brightWhite: '#c0caf5'
  },
  Dracula: {
    background: '#282a36', foreground: '#f8f8f2', cursor: '#f8f8f2', selectionBackground: '#44475a',
    black: '#21222c', red: '#ff5555', green: '#50fa7b', yellow: '#f1fa8c',
    blue: '#bd93f9', magenta: '#ff79c6', cyan: '#8be9fd', white: '#f8f8f2',
    brightBlack: '#6272a4', brightRed: '#ff6e6e', brightGreen: '#69ff94', brightYellow: '#ffffa5',
    brightBlue: '#d6acff', brightMagenta: '#ff92df', brightCyan: '#a4ffff', brightWhite: '#ffffff'
  },
  'One Dark': {
    background: '#282c34', foreground: '#abb2bf', cursor: '#528bff', selectionBackground: '#3e4451',
    black: '#282c34', red: '#e06c75', green: '#98c379', yellow: '#e5c07b',
    blue: '#61afef', magenta: '#c678dd', cyan: '#56b6c2', white: '#abb2bf',
    brightBlack: '#5c6370', brightRed: '#e06c75', brightGreen: '#98c379', brightYellow: '#e5c07b',
    brightBlue: '#61afef', brightMagenta: '#c678dd', brightCyan: '#56b6c2', brightWhite: '#ffffff'
  },
  Nord: {
    background: '#2e3440', foreground: '#d8dee9', cursor: '#d8dee9', selectionBackground: '#434c5e',
    black: '#3b4252', red: '#bf616a', green: '#a3be8c', yellow: '#ebcb8b',
    blue: '#81a1c1', magenta: '#b48ead', cyan: '#88c0d0', white: '#e5e9f0',
    brightBlack: '#4c566a', brightRed: '#bf616a', brightGreen: '#a3be8c', brightYellow: '#ebcb8b',
    brightBlue: '#81a1c1', brightMagenta: '#b48ead', brightCyan: '#8fbcbb', brightWhite: '#eceff4'
  },
  'Gruvbox Dark': {
    background: '#282828', foreground: '#ebdbb2', cursor: '#ebdbb2', selectionBackground: '#504945',
    black: '#282828', red: '#cc241d', green: '#98971a', yellow: '#d79921',
    blue: '#458588', magenta: '#b16286', cyan: '#689d6a', white: '#a89984',
    brightBlack: '#928374', brightRed: '#fb4934', brightGreen: '#b8bb26', brightYellow: '#fabd2f',
    brightBlue: '#83a598', brightMagenta: '#d3869b', brightCyan: '#8ec07c', brightWhite: '#ebdbb2'
  },
  'Catppuccin Mocha': {
    background: '#1e1e2e', foreground: '#cdd6f4', cursor: '#f5e0dc', selectionBackground: '#585b70',
    black: '#45475a', red: '#f38ba8', green: '#a6e3a1', yellow: '#f9e2af',
    blue: '#89b4fa', magenta: '#f5c2e7', cyan: '#94e2d5', white: '#bac2de',
    brightBlack: '#585b70', brightRed: '#f38ba8', brightGreen: '#a6e3a1', brightYellow: '#f9e2af',
    brightBlue: '#89b4fa', brightMagenta: '#f5c2e7', brightCyan: '#94e2d5', brightWhite: '#a6adc8'
  },
  Monokai: {
    background: '#272822', foreground: '#f8f8f2', cursor: '#f8f8f0', selectionBackground: '#49483e',
    black: '#272822', red: '#f92672', green: '#a6e22e', yellow: '#f4bf75',
    blue: '#66d9ef', magenta: '#ae81ff', cyan: '#a1efe4', white: '#f8f8f2',
    brightBlack: '#75715e', brightRed: '#f92672', brightGreen: '#a6e22e', brightYellow: '#f4bf75',
    brightBlue: '#66d9ef', brightMagenta: '#ae81ff', brightCyan: '#a1efe4', brightWhite: '#f9f8f5'
  },
  'Night Owl': {
    background: '#011627', foreground: '#d6deeb', cursor: '#80a4c2', selectionBackground: '#1d3b53',
    black: '#011627', red: '#ef5350', green: '#22da6e', yellow: '#addb67',
    blue: '#82aaff', magenta: '#c792ea', cyan: '#21c7a8', white: '#ffffff',
    brightBlack: '#575656', brightRed: '#ef5350', brightGreen: '#22da6e', brightYellow: '#ffeb95',
    brightBlue: '#82aaff', brightMagenta: '#c792ea', brightCyan: '#7fdbca', brightWhite: '#ffffff'
  },
  Palenight: {
    background: '#292d3e', foreground: '#a6accd', cursor: '#ffcc00', selectionBackground: '#444267',
    black: '#292d3e', red: '#f07178', green: '#c3e88d', yellow: '#ffcb6b',
    blue: '#82aaff', magenta: '#c792ea', cyan: '#89ddff', white: '#d0d0d0',
    brightBlack: '#434758', brightRed: '#ff8b92', brightGreen: '#ddffa7', brightYellow: '#ffe585',
    brightBlue: '#9cc4ff', brightMagenta: '#e1acff', brightCyan: '#a3f7ff', brightWhite: '#ffffff'
  },
  'Ayu Dark': {
    background: '#0a0e14', foreground: '#b3b1ad', cursor: '#e6b450', selectionBackground: '#273747',
    black: '#01060e', red: '#ea6c73', green: '#91b362', yellow: '#f9af4f',
    blue: '#53bdfa', magenta: '#d2a6ff', cyan: '#90e1c6', white: '#c7c7c7',
    brightBlack: '#686868', brightRed: '#f07178', brightGreen: '#c2d94c', brightYellow: '#ffb454',
    brightBlue: '#59c2ff', brightMagenta: '#ffee99', brightCyan: '#95e6cb', brightWhite: '#ffffff'
  },
  'GitHub Dark': {
    background: '#0d1117', foreground: '#c9d1d9', cursor: '#c9d1d9', selectionBackground: '#163356',
    black: '#484f58', red: '#ff7b72', green: '#3fb950', yellow: '#d29922',
    blue: '#58a6ff', magenta: '#bc8cff', cyan: '#39c5cf', white: '#b1bac4',
    brightBlack: '#6e7681', brightRed: '#ffa198', brightGreen: '#56d364', brightYellow: '#e3b341',
    brightBlue: '#79c0ff', brightMagenta: '#d2a8ff', brightCyan: '#56d4dd', brightWhite: '#f0f6fc'
  },
  'Rosé Pine': {
    background: '#191724', foreground: '#e0def4', cursor: '#e0def4', selectionBackground: '#403d52',
    black: '#26233a', red: '#eb6f92', green: '#3e8fb0', yellow: '#f6c177',
    blue: '#9ccfd8', magenta: '#c4a7e7', cyan: '#ebbcba', white: '#e0def4',
    brightBlack: '#6e6a86', brightRed: '#eb6f92', brightGreen: '#3e8fb0', brightYellow: '#f6c177',
    brightBlue: '#9ccfd8', brightMagenta: '#c4a7e7', brightCyan: '#ebbcba', brightWhite: '#e0def4'
  },
  'Solarized Dark': {
    background: '#002b36', foreground: '#839496', cursor: '#93a1a1', selectionBackground: '#073642',
    black: '#073642', red: '#dc322f', green: '#859900', yellow: '#b58900',
    blue: '#268bd2', magenta: '#d33682', cyan: '#2aa198', white: '#eee8d5',
    brightBlack: '#586e75', brightRed: '#cb4b16', brightGreen: '#586e75', brightYellow: '#657b83',
    brightBlue: '#839496', brightMagenta: '#6c71c4', brightCyan: '#93a1a1', brightWhite: '#fdf6e3'
  },
  'Solarized Light': {
    background: '#fdf6e3', foreground: '#657b83', cursor: '#586e75', selectionBackground: '#eee8d5',
    black: '#073642', red: '#dc322f', green: '#859900', yellow: '#b58900',
    blue: '#268bd2', magenta: '#d33682', cyan: '#2aa198', white: '#eee8d5',
    brightBlack: '#93a1a1', brightRed: '#cb4b16', brightGreen: '#586e75', brightYellow: '#657b83',
    brightBlue: '#268bd2', brightMagenta: '#6c71c4', brightCyan: '#2aa198', brightWhite: '#fdf6e3'
  },
  'One Light': {
    background: '#fafafa', foreground: '#383a42', cursor: '#526fff', selectionBackground: '#e5e5e6',
    black: '#383a42', red: '#e45649', green: '#50a14f', yellow: '#c18401',
    blue: '#4078f2', magenta: '#a626a4', cyan: '#0184bc', white: '#fafafa',
    brightBlack: '#a0a1a7', brightRed: '#e45649', brightGreen: '#50a14f', brightYellow: '#c18401',
    brightBlue: '#4078f2', brightMagenta: '#a626a4', brightCyan: '#0184bc', brightWhite: '#ffffff'
  },
  'Catppuccin Latte': {
    background: '#eff1f5', foreground: '#4c4f69', cursor: '#dc8a78', selectionBackground: '#ccced7',
    black: '#5c5f77', red: '#d20f39', green: '#40a02b', yellow: '#df8e1d',
    blue: '#1e66f5', magenta: '#ea76cb', cyan: '#179299', white: '#acb0be',
    brightBlack: '#8c8fa1', brightRed: '#d20f39', brightGreen: '#40a02b', brightYellow: '#df8e1d',
    brightBlue: '#1e66f5', brightMagenta: '#ea76cb', brightCyan: '#179299', brightWhite: '#bcc0cc'
  }
}

export const THEME_NAMES = Object.keys(THEMES)

export function getTheme(name: string): ITheme {
  return THEMES[name] ?? THEMES['Tokyo Night']
}

// ---------- Применение темы ко всему UI (CSS-переменные) ----------

function hexToRgb(h: string): [number, number, number] {
  let s = h.replace('#', '')
  if (s.length === 3) s = s.split('').map((c) => c + c).join('')
  const n = parseInt(s, 16)
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255]
}
function rgbToHex([r, g, b]: [number, number, number]): string {
  return '#' + [r, g, b].map((v) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, '0')).join('')
}
function mix(a: string, b: string, t: number): string {
  const A = hexToRgb(a)
  const B = hexToRgb(b)
  return rgbToHex([A[0] + (B[0] - A[0]) * t, A[1] + (B[1] - A[1]) * t, A[2] + (B[2] - A[2]) * t])
}
function luminance(h: string): number {
  const [r, g, b] = hexToRgb(h).map((v) => v / 255)
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

function rgbTriple(h: string): string {
  return hexToRgb(h).join(', ')
}

/** Выводит палитру UI из цветовой схемы терминала и применяет к CSS-переменным. */
export function applyUiTheme(name: string): void {
  const t = getTheme(name)
  const bg = t.background ?? '#1a1b26'
  const fg = t.foreground ?? '#c0caf5'
  const accent = t.brightBlue ?? t.blue ?? '#7aa2f7'
  const danger = t.red ?? '#f7768e'
  const green = t.green ?? '#9ece6a'
  const dark = luminance(bg) < 0.5

  const vars: Record<string, string> = {
    '--bg': bg,
    '--bg-alt': dark ? mix(bg, '#000000', 0.3) : mix(bg, '#000000', 0.05),
    '--panel': mix(bg, fg, dark ? 0.08 : 0.06),
    '--panel-2': mix(bg, fg, dark ? 0.14 : 0.11),
    // Приподнятая поверхность для всплывающих панелей/меню/палитры — на тон выше panel-2.
    '--elevated': dark ? mix(bg, fg, 0.15) : mix(bg, '#ffffff', 0.5),
    '--border': mix(bg, fg, dark ? 0.2 : 0.17),
    // Тонкая линия-«хайлайт» сверху поверхностей (имитация фаски/освещения).
    '--hairline': dark ? mix(bg, fg, 0.32) : mix(bg, '#ffffff', 0.7),
    '--text': fg,
    // Приглушённый текст — выводим из bg↔fg, чтобы гарантировать читаемый контраст
    // на всех поверхностях (raw brightBlack у части тем слишком тёмный → сливался).
    '--muted': mix(bg, fg, dark ? 0.56 : 0.5),
    '--accent': accent,
    // Чуть высветленный акцент — для градиентов и свечения.
    '--accent-2': mix(accent, dark ? '#ffffff' : fg, 0.28),
    '--danger': danger,
    '--green': green,
    // RGB-тройки: позволяют делать тематические полупрозрачные заливки rgba(var(--accent-rgb), .12).
    '--accent-rgb': rgbTriple(accent),
    '--danger-rgb': rgbTriple(danger),
    '--green-rgb': rgbTriple(green),
    '--fg-rgb': rgbTriple(fg),
    // Цвет тени: на тёмных темах — чёрный, на светлых — холодный графит (мягче).
    '--shadow-rgb': dark ? '0, 0, 0' : rgbTriple(mix(bg, '#1a2030', 0.85)),
    // Контрастный цвет текста на акцентных кнопках.
    '--on-accent': luminance(accent) > 0.55 ? '#15161e' : '#ffffff'
  }
  const root = document.documentElement
  for (const [k, v] of Object.entries(vars)) root.style.setProperty(k, v)
  // Маркер светлой темы — для редких случаев, где нужно по-разному вести себя на светлом фоне.
  root.dataset.themeMode = dark ? 'dark' : 'light'
}
