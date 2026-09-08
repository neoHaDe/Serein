/**
 * Что за файл, судя по имени.
 *
 * Отдельным модулем намеренно: этими проверками пользуется файловый менеджер, а рядом
 * с ними раньше жил `languageFor`, тянущий за собой весь CodeMirror с грамматиками.
 * Две проверки расширения в SFTP затаскивали редактор в стартовый чанк целиком.
 * Здесь импортов нет и не должно появиться.
 */

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

/** Расширение в нижнем регистре, без точки. Пусто, если его нет. */
export function extOf(name: string): string {
  const i = name.lastIndexOf('.')
  return i > 0 ? name.slice(i + 1).toLowerCase() : ''
}

const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'ico', 'svg'])

/** Картинка - превью во вкладке, не скачивание и не текстовый редактор. */
export function isImageFile(name: string): boolean {
  return IMAGE_EXTS.has(extOf(name))
}

/** Похоже ли имя файла на текстовый файл (для открытия в редакторе). */
export function isTextFile(name: string): boolean {
  const lower = name.toLowerCase()
  if (TEXT_NAMES.has(lower)) return true
  // Dockerfile.dev, Dockerfile.prod и т.п.
  if (lower.startsWith('dockerfile')) return true
  return TEXT_EXTS.has(extOf(name))
}
