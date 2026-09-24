import { describe, expect, it } from 'vitest'
import {
  filterEntries,
  fmtEta,
  fmtMode,
  fmtSize,
  fmtSpeed,
  isSereinDnd,
  joinLocal,
  joinRemote,
  localBaseName,
  localProps,
  nextRate,
  parentOfAny,
  parentOfRemote,
  remoteCrumbs,
  remoteProps,
} from './sftpPaths'

describe('пути панели файлов', () => {
  it('крошки строятся только для абсолютного пути', () => {
    expect(remoteCrumbs('.')).toEqual([])
    expect(remoteCrumbs('/srv/app')).toEqual([
      { label: '/', path: '/' },
      { label: 'srv', path: '/srv' },
      { label: 'app', path: '/srv/app' },
    ])
  })

  it('родитель и соединение путей сервера', () => {
    expect(parentOfRemote('/srv/app/')).toBe('/srv')
    expect(parentOfRemote('/srv')).toBe('/')
    expect(parentOfRemote('')).toBe('/')
    expect(joinRemote('/', 'a')).toBe('/a')
    expect(joinRemote('/srv', 'a')).toBe('/srv/a')
  })

  it('локальный путь соединяется тем разделителем, что уже в нём', () => {
    expect(joinLocal('C:\\Users\\u', 'f.txt')).toBe('C:\\Users\\u\\f.txt')
    expect(joinLocal('C:\\', 'f.txt')).toBe('C:\\f.txt')
    expect(joinLocal('/home/u', 'f.txt')).toBe('/home/u/f.txt')
    expect(localBaseName('C:\\Users\\u\\папка\\')).toBe('папка')
  })

  it('повтор передачи идёт в тот же каталог на любой системе', () => {
    expect(parentOfAny('C:\\Загрузки\\отчёт.pdf')).toBe('C:\\Загрузки')
    expect(parentOfAny('/srv/www/index.html')).toBe('/srv/www')
    expect(parentOfAny('file')).toBe('file')
  })

  it('скрытые файлы и поиск', () => {
    const e = [{ name: '.env' }, { name: 'app.js' }, { name: '..' }, { name: 'README' }]
    expect(filterEntries(e, false, '').map((x) => x.name)).toEqual(['app.js', '..', 'README'])
    expect(filterEntries(e, true, 'EN').map((x) => x.name)).toEqual(['.env'])
  })

  it('временный каталог перетаскивания узнаётся на любой системе', () => {
    expect(isSereinDnd('C:\\Users\\u\\AppData\\Local\\Temp\\serein-dnd\\x\\f')).toBe(true)
    expect(isSereinDnd('C:/Users/u/Downloads/f')).toBe(false)
  })
})

describe('форматы', () => {
  it('размер, скорость, права', () => {
    expect(fmtSize(512)).toBe('512 Б')
    expect(fmtSize(1536)).toBe('1.5 КБ')
    expect(fmtSize(3 * 1024 ** 3)).toBe('3.00 ГБ')
    expect(fmtSpeed(100)).toBe('')
    expect(fmtSpeed(2048)).toBe('2.0 КБ/s')
    expect(fmtMode(0o100644)).toBe('644')
  })

  it('оставшееся время показывается, только когда его есть что показать', () => {
    expect(fmtEta(100, 100, 5000)).toBe('')
    expect(fmtEta(10 * 1024 * 1024, 0, 1024 * 1024)).toBe('10с')
    expect(fmtEta(200 * 1024 * 1024, 0, 1024 * 1024)).toBe('3м 20с')
    expect(fmtEta(1000, 0, 0)).toBe('')
  })
})

describe('скорость передачи', () => {
  it('первый замер - ноль, потом сглаживание', () => {
    const a = nextRate(undefined, 'active', 0, 1000)
    expect(a).toEqual({ bps: 0, sample: { t: 1000, b: 0, bps: 0 } })
    const b = nextRate(a.sample!, 'active', 1000, 2000)
    expect(b.bps).toBe(1000)
    const c = nextRate(b.sample!, 'active', 4000, 3000)
    expect(c.bps).toBeCloseTo(1000 * 0.55 + 3000 * 0.45)
  })

  it('частые события не дёргают цифру, а конец передачи забывает замер', () => {
    const prev = { t: 1000, b: 100, bps: 500 }
    expect(nextRate(prev, 'active', 200, 1100)).toEqual({ bps: 500, sample: undefined })
    expect(nextRate(prev, 'active', 100, 5000)).toEqual({ bps: 500, sample: undefined })
    expect(nextRate(prev, 'done', 200, 5000)).toEqual({ bps: 0, sample: null })
  })
})

describe('лист «Свойства»', () => {
  const file = { name: 'app.log', type: 'file' as const, size: 2048, mtime: 0, mode: 0o100640 }
  const link = { name: 'cur', type: 'link' as const, size: 0, mtime: 0, mode: 0o120777, target: '/srv/v2', linkType: 'dir' as const }

  it('одна запись на сервере - подробно, со ссылкой и правами', () => {
    const one = remoteProps([link], '/srv')
    expect(one.title).toBe('Свойства')
    expect(one.rows.find((r) => r.k === 'Ссылка')?.v).toBe('/srv/v2')
    expect(remoteProps([file], '/var/log').rows.find((r) => r.k === 'Права')?.v).toBe('rw-r----- (640)')
  })

  it('несколько - сводкой: ссылка на каталог считается папкой', () => {
    const many = remoteProps([file, link], '/srv')
    expect(many.title).toBe('2 элементов')
    expect(many.rows.find((r) => r.k === 'Папок')?.v).toBe('1')
    expect(many.rows.find((r) => r.k === 'Суммарный размер')?.v).toBe('2.0 КБ')
  })

  it('своя машина - без прав', () => {
    const one = localProps([{ name: 'a.txt', type: 'file', size: 1, mtime: 0 }], 'C:\\')
    expect(one.rows.map((r) => r.k)).not.toContain('Права')
    expect(localProps([{ name: 'd', type: 'dir', size: 0, mtime: 0 }, { name: 'f', type: 'file', size: 3, mtime: 0 }], 'C:\\').rows[1].v).toBe('1')
  })
})
