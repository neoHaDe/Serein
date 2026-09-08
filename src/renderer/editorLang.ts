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

import { extOf } from './fileKind'

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
