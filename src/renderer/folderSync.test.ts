import { describe, it, expect } from 'vitest'
import {
  changedSincePlan,
  countKinds,
  joinLocalRel,
  toUpload,
  uploadSteps,
  type SyncItem,
  type SyncPlan
} from './folderSync'

const item = (rel: string, kind: SyncItem['kind']): SyncItem => ({ rel, kind })

const plan = (items: SyncItem[], remoteDirs: string[] = [], localRoot = '/home/me/site'): SyncPlan => ({
  localRoot,
  remoteRoot: '/var/www/site',
  timeKnown: true,
  items,
  remoteDirs,
  refused: []
})

describe('сравнение папок', () => {
  it('считает файлы по видам', () => {
    const c = countKinds([item('a', 'new'), item('b', 'new'), item('c', 'same'), item('d', 'remoteNewer')])
    expect(c).toEqual({ changed: 0, new: 2, remoteNewer: 1, unsure: 0, remoteOnly: 0, same: 1 })
  })

  it('«на сервере новее» заливается только по явному согласию', () => {
    const items = [item('a', 'changed'), item('b', 'remoteNewer'), item('c', 'remoteOnly'), item('d', 'same')]
    expect(toUpload(items, false).map((i) => i.rel)).toEqual(['a'])
    expect(toUpload(items, true).map((i) => i.rel)).toEqual(['a', 'b'])
  })

  it('создаёт недостающие каталоги от внешних к внутренним и группирует заливку', () => {
    const steps = uploadSteps(
      plan([item('index.html', 'changed'), item('css/new/a.css', 'new'), item('css/b.css', 'new')], ['css']),
      false
    )
    expect(steps.mkdirs).toEqual(['/var/www/site/css/new'])
    expect(steps.files).toBe(3)
    expect(steps.groups).toEqual([
      { remoteDir: '/var/www/site', localPaths: ['/home/me/site/index.html'] },
      { remoteDir: '/var/www/site/css/new', localPaths: ['/home/me/site/css/new/a.css'] },
      { remoteDir: '/var/www/site/css', localPaths: ['/home/me/site/css/b.css'] }
    ])
    const deep = uploadSteps(plan([item('a/b/c.txt', 'new')]), false)
    expect(deep.mkdirs).toEqual(['/var/www/site/a', '/var/www/site/a/b'])
  })

  it('путь своего файла на Windows собирается обратной чертой', () => {
    expect(joinLocalRel('C:\\Users\\me\\site\\', 'css/a.css')).toBe('C:\\Users\\me\\site\\css\\a.css')
    expect(joinLocalRel('/home/me/site', 'css/a.css')).toBe('/home/me/site/css/a.css')
  })

  it('«не определить» заливается только по явному согласию', () => {
    const items = [item('a', 'unsure'), item('b', 'changed')]
    expect(toUpload(items, false).map((i) => i.rel)).toEqual(['b'])
    expect(toUpload(items, false, true).map((i) => i.rel)).toEqual(['a', 'b'])
    expect(uploadSteps(plan(items), false, true).files).toBe(2)
  })

  it('по старому плану не заливает то, что на сервере успели поменять', () => {
    const was: SyncItem = { rel: 'app.conf', kind: 'changed', remoteSize: 10, remoteMtime: 1000 }
    expect(changedSincePlan([was], plan([{ ...was }]))).toEqual([])
    expect(changedSincePlan([was], plan([{ ...was, kind: 'remoteNewer', remoteMtime: 5000 }]))).toEqual(['app.conf'])
    expect(changedSincePlan([was], plan([{ ...was, remoteSize: 11 }]))).toEqual(['app.conf'])
    const added: SyncItem = { rel: 'new.txt', kind: 'new' }
    expect(changedSincePlan([added], plan([{ ...added, kind: 'changed', remoteSize: 3, remoteMtime: 7 }]))).toEqual([
      'new.txt'
    ])
    expect(changedSincePlan([was], plan([]))).toEqual(['app.conf'])
  })
})
