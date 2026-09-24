import type { LocalEntry, SftpEntry } from '../../shared/types'
import { isImageFile, isTextFile } from '../fileKind'
import { isFileLike } from '../sftpPaths'
import { AnchoredMenu, CtxItem } from './SftpParts'

/** Подпись пункта: для нескольких записей - с числом в скобках. */
function many(n: number, one: string, several: string): string {
  return n > 1 ? `${several} (${n})` : one
}

/** Контекстное меню выделенного на сервере. Любой пункт сначала закрывает меню. */
export function RemoteMenu({
  x,
  y,
  items,
  dualPane,
  canBuiltin,
  onClose,
  act,
}: {
  x: number
  y: number
  items: SftpEntry[]
  dualPane: boolean
  /** Есть ли встроенный редактор (в откреплённом окне его нет). */
  canBuiltin: boolean
  onClose: () => void
  act: {
    open: (items: SftpEntry[]) => void
    openBuiltin: (items: SftpEntry[]) => void
    openExternal: (items: SftpEntry[]) => void
    download: (items: SftpEntry[]) => void
    downloadToLocal: (items: SftpEntry[]) => void
    rename: (entry: SftpEntry) => void
    remove: (items: SftpEntry[]) => void
    properties: () => void
  }
}): JSX.Element {
  const builtin = items.filter((e) => isFileLike(e) && (isTextFile(e.name) || isImageFile(e.name)))
  const external = items.filter((e) => isFileLike(e) && !isImageFile(e.name))
  const files = items.filter(isFileLike)
  const pick = (f: () => void) => () => {
    onClose()
    f()
  }
  return (
    <AnchoredMenu className="sftp-ctx-menu" x={x} y={y}>
      <CtxItem label={many(items.length, 'Открыть', 'Открыть')} onPick={pick(() => act.open(items))} />
      {builtin.length > 0 && canBuiltin ? (
        <CtxItem
          label={many(builtin.length, 'Открыть во встроенном редакторе', 'Во встроенном редакторе')}
          onPick={pick(() => act.openBuiltin(builtin))}
        />
      ) : null}
      {external.length > 0 ? (
        <CtxItem
          label={many(external.length, 'Открыть во внешнем редакторе', 'Во внешнем редакторе')}
          onPick={pick(() => act.openExternal(external))}
        />
      ) : null}
      <CtxItem label={many(items.length, 'Скачать', 'Скачать')} onPick={pick(() => act.download(items))} />
      {dualPane && files.length > 0 ? (
        <CtxItem label="Скачать на этот компьютер" onPick={pick(() => act.downloadToLocal(files))} />
      ) : null}
      <div className="sftp-ctx-sep" />
      {items.length === 1 ? <CtxItem label="Переименовать" onPick={pick(() => act.rename(items[0]))} /> : null}
      <CtxItem label={many(items.length, 'Удалить', 'Удалить')} danger onPick={pick(() => act.remove(items))} />
      <div className="sftp-ctx-sep" />
      <CtxItem label="Свойства" onPick={pick(act.properties)} />
    </AnchoredMenu>
  )
}

/** Контекстное меню выделенного на своей машине (двухпанельный режим). */
export function LocalMenu({
  x,
  y,
  items,
  onClose,
  act,
}: {
  x: number
  y: number
  items: LocalEntry[]
  onClose: () => void
  act: {
    openDir: (entry: LocalEntry) => void
    upload: (items: LocalEntry[]) => void
    properties: () => void
  }
}): JSX.Element {
  const pick = (f: () => void) => () => {
    onClose()
    f()
  }
  return (
    <AnchoredMenu className="sftp-ctx-menu" x={x} y={y}>
      {items.length === 1 && items[0].type === 'dir' ? (
        <CtxItem label="Открыть" onPick={pick(() => act.openDir(items[0]))} />
      ) : null}
      {items.some((e) => e.type !== 'dir') ? (
        <CtxItem
          label={many(items.length, 'Загрузить на сервер', 'Загрузить на сервер')}
          onPick={pick(() => act.upload(items))}
        />
      ) : null}
      <div className="sftp-ctx-sep" />
      <CtxItem label="Свойства" onPick={pick(act.properties)} />
    </AnchoredMenu>
  )
}
