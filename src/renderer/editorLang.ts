import type { Extension } from '@codemirror/state'
import { StreamLanguage } from '@codemirror/language'
import { python } from '@codemirror/lang-python'
import { javascript } from '@codemirror/lang-javascript'
import { json } from '@codemirror/lang-json'
import { yaml } from '@codemirror/lang-yaml'
import { markdown } from '@codemirror/lang-markdown'
import { html } from '@codemirror/lang-html'
import { css } from '@codemirror/lang-css'
import { dockerFile } from '@codemirror/legacy-modes/mode/dockerfile'
import { shell } from '@codemirror/legacy-modes/mode/shell'
import { properties } from '@codemirror/legacy-modes/mode/properties'
import { nginx } from '@codemirror/legacy-modes/mode/nginx'
import { toml } from '@codemirror/legacy-modes/mode/toml'

/** Расширения, считающиеся текстовыми (можно открыть во встроенном редакторе). */
const TEXT_EXTS = new Set([
  'txt', 'md', 'markdown', 'log', 'py', 'js', 'jsx', 'ts', 'tsx', 'mjs', 'cjs',
  'json', 'json5', 'yaml', 'yml', 'toml', 'ini', 'cfg', 'conf', 'config', 'env',
  'sh', 'bash', 'zsh', 'fish', 'ps1', 'bat', 'cmd', 'html', 'htm', 'xml', 'svg',
  'css', 'scss', 'less', 'sql', 'go', 'rs', 'rb', 'php', 'pl', 'lua', 'c', 'h',
  'cpp', 'hpp', 'cc', 'java', 'kt', 'gradle', 'properties', 'gitignore',
  'dockerignore', 'editorconfig', 'csv', 'tsv', 'nginx', 'service', 'tf', 'vue'
])

/** Имена файлов без расширения, которые тоже текстовые. */
const TEXT_NAMES = new Set([
  'dockerfile', 'makefile', 'jenkinsfile', 'vagrantfile', 'procfile',
  '.gitignore', '.dockerignore', '.env', '.bashrc', '.bash_profile', '.profile',
  '.zshrc', '.vimrc', '.editorconfig', 'readme', 'license', 'changelog'
])

function extOf(name: string): string {
  const i = name.lastIndexOf('.')
  return i > 0 ? name.slice(i + 1).toLowerCase() : ''
}

/** Похоже ли имя файла на текстовый файл (для открытия в редакторе). */
export function isTextFile(name: string): boolean {
  const lower = name.toLowerCase()
  if (TEXT_NAMES.has(lower)) return true
  // Dockerfile.dev, Dockerfile.prod и т.п.
  if (lower.startsWith('dockerfile')) return true
  return TEXT_EXTS.has(extOf(name))
}

/** Возвращает language-расширение CodeMirror по имени файла (или undefined). */
export function languageFor(name: string): Extension | undefined {
  const lower = name.toLowerCase()
  if (lower === 'dockerfile' || lower.startsWith('dockerfile')) return StreamLanguage.define(dockerFile)
  if (lower === 'nginx.conf' || lower.endsWith('.nginx')) return StreamLanguage.define(nginx)

  switch (extOf(name)) {
    case 'py':
      return python()
    case 'js':
    case 'jsx':
    case 'mjs':
    case 'cjs':
    case 'vue':
      return javascript()
    case 'ts':
    case 'tsx':
      return javascript({ typescript: true, jsx: true })
    case 'json':
    case 'json5':
      return json()
    case 'yaml':
    case 'yml':
      return yaml()
    case 'toml':
      return StreamLanguage.define(toml)
    case 'md':
    case 'markdown':
      return markdown()
    case 'html':
    case 'htm':
    case 'xml':
    case 'svg':
      return html()
    case 'css':
    case 'scss':
    case 'less':
      return css()
    case 'sh':
    case 'bash':
    case 'zsh':
    case 'fish':
    case 'env':
      return StreamLanguage.define(shell)
    case 'ini':
    case 'cfg':
    case 'conf':
    case 'config':
    case 'properties':
    case 'editorconfig':
      return StreamLanguage.define(properties)
    default:
      return undefined
  }
}
