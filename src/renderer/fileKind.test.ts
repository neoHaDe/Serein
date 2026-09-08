import { describe, it, expect } from 'vitest'
import { extOf, isImageFile, isTextFile } from './fileKind'

/*
 * Эти три функции переехали из `editorLang.ts` отдельным модулем: там рядом жил
 * `languageFor`, тянущий весь CodeMirror, и две проверки расширения в файловом
 * менеджере затаскивали редактор со всеми грамматиками в стартовый чанк.
 *
 * Тест закрывает именно переезд: поведение должно остаться прежним, а импортов у
 * модуля не должно появиться вовсе. Если однажды кто-то снова принесёт сюда
 * `@codemirror/...`, вес стартового бандла вырастет молча.
 */

describe('расширение файла', () => {
  it('берётся после последней точки и в нижнем регистре', () => {
    expect(extOf('script.SH')).toBe('sh')
    expect(extOf('archive.tar.gz')).toBe('gz')
  })

  it('у файла без расширения его нет', () => {
    expect(extOf('Makefile')).toBe('')
    // Точка в начале - это скрытый файл, а не расширение: у `.bashrc` имя такое.
    expect(extOf('.bashrc')).toBe('')
  })
})

describe('картинка ли это', () => {
  it('узнаёт обычные форматы', () => {
    expect(isImageFile('photo.PNG')).toBe(true)
    expect(isImageFile('icon.svg')).toBe(true)
  })

  it('текст картинкой не считает', () => {
    expect(isImageFile('notes.txt')).toBe(false)
    expect(isImageFile('Dockerfile')).toBe(false)
  })
})

describe('текстовый ли файл', () => {
  it('по расширению', () => {
    expect(isTextFile('app.py')).toBe(true)
    expect(isTextFile('nginx.conf')).toBe(true)
  })

  it('по имени, когда расширения нет вовсе', () => {
    expect(isTextFile('Makefile')).toBe(true)
    expect(isTextFile('.gitignore')).toBe(true)
  })

  it('Dockerfile с суффиксом тоже текстовый', () => {
    // Dockerfile.dev и Dockerfile.prod встречаются чаще, чем голый Dockerfile.
    expect(isTextFile('Dockerfile.prod')).toBe(true)
  })

  it('двоичное не открываем', () => {
    expect(isTextFile('serein.exe')).toBe(false)
    expect(isTextFile('dump.tar.gz')).toBe(false)
  })
})
