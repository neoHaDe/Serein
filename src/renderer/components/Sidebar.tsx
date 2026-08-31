import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { ServerConfig } from '../../shared/types'
import { Icon } from './Icon'
import { ContextMenu, type MenuItem } from './ContextMenu'
import { ENVS, ENV_LABEL, filterServers } from '../serverFilter'
import {
  EMPTY_SELECTION,
  clickSelect,
  isSelected,
  pruneSelection,
  targetsFor,
  type Selection
} from '../selection'

/** Цвет точки по агрегированному статусу подключения сервера. */
const STATUS_DOT: Record<string, string> = {
  connected: 'var(--green)',
  connecting: '#e0af68',
  reconnecting: '#e0af68',
  error: 'var(--danger)'
}

/** Псевдогруппа для серверов без группы — всегда последняя в списке. */
const UNGROUPED = ''
const UNGROUPED_TITLE = 'Без группы'

/** Сколько пикселей нужно протащить, чтобы это считалось перетаскиванием, а не кликом. */
const DRAG_THRESHOLD = 4
/** Зона у края списка, в которой он подкручивается сам. */
const EDGE = 48

interface Props {
  servers: ServerConfig[]
  onConnect: (s: ServerConfig) => void
  onOpenLocal: () => void
  onNew: () => void
  onEdit: (s: ServerConfig) => void
  onDelete: (id: string) => void
  onOpenSettings: () => void
  onOpenKeyGen: () => void
  onImport: (kind: 'ssh' | 'putty' | 'mobaxterm' | 'xshell' | 'securecrt') => void
  showPuttyImport?: boolean
  width: number
  collapsed?: boolean
  onToggleCollapse?: () => void
  statuses?: Record<string, 'connected' | 'connecting' | 'reconnecting' | 'error'>
  /** Порядок групп; пустые группы тоже здесь — иначе они бы исчезали. */
  groupOrder: string[]
  collapsedGroups: string[]
  onToggleGroup: (group: string) => void
  onNewGroup: () => void
  onOpenGroups: () => void
  /** Выполнить одну команду сразу на нескольких серверах. */
  onMultiExec: () => void
  /** Точечная правка профиля из списка: избранное и метка среды. */
  onPatch?: (id: string, patch: Partial<ServerConfig>) => void
  /** Перетаскивание: сервер попал в группу на позицию `index` (в конец, если undefined). */
  onDropServer: (serverId: string, group: string, index?: number) => void
  /** Новый порядок групп целиком — сайдбар знает, что именно нарисовал. */
  onDropGroup: (order: string[]) => void
}

/** Подпись под именем: у SSH — user@host, у COM-порта — порт и скорость. */
/** Подсказка строки: к имени и адресу добавляются среда и теги, если они заданы. */
function serverTooltip(s: ServerConfig): string {
  const parts = [`${s.name} — ${serverSubtitle(s)}`]
  if (s.env) parts.push(`Среда: ${ENV_LABEL[s.env]}`)
  const tags = s.tags ?? []
  if (tags.length > 0) parts.push(`Теги: ${tags.join(', ')}`)
  return parts.join(' · ')
}

function serverSubtitle(s: ServerConfig): string {
  if (s.connection === 'serial') {
    const cfg = s.serial
    return cfg ? `${cfg.port} · ${cfg.baudRate} бод` : 'COM-порт не настроен'
  }
  // У telnet и сырого TCP пользователя нет — показываем адрес с портом.
  if (s.connection === 'telnet') return `telnet ${s.host}:${s.port}`
  if (s.connection === 'raw') return `TCP ${s.host}:${s.port}`
  return `${s.username}@${s.host}`
}

/** Строка списка, измеренная в момент старта перетаскивания. */
interface Row {
  key: string
  kind: 'header' | 'server' | 'empty'
  group: string
  /** Индекс сервера внутри группы; у заголовков и заглушек — -1. */
  index: number
  /** Координаты в системе содержимого списка (с учётом прокрутки). */
  top: number
  height: number
}

interface Drag {
  kind: 'server' | 'group'
  /** id сервера либо имя группы. */
  id: string
  group: string
  index: number
  /** Высота того, что двигаем: на неё расступаются соседи. */
  step: number
  /** Геометрия элемента на момент захвата — чтобы он точно шёл за курсором. */
  height: number
  top: number
  grabY: number
}

/** Куда встанет сервер, если отпустить сейчас. */
interface Target {
  group: string
  index: number
}

interface Press {
  kind: 'server' | 'group'
  id: string
  group: string
  index: number
  el: HTMLElement
  x: number
  y: number
}

export function Sidebar({
  servers,
  onConnect,
  onOpenLocal,
  onNew,
  onEdit,
  onDelete,
  onOpenSettings,
  onOpenKeyGen,
  onImport,
  showPuttyImport = true,
  width,
  collapsed,
  onToggleCollapse,
  statuses,
  groupOrder,
  collapsedGroups,
  onToggleGroup,
  onNewGroup,
  onOpenGroups,
  onMultiExec,
  onPatch,
  onDropServer,
  onDropGroup
}: Props): JSX.Element {
  const [filter, setFilter] = useState('')
  /** Выделение для групповых действий. Правила — в selection.ts, там же тесты. */
  const [sel, setSel] = useState<Selection>(EMPTY_SELECTION)
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null)
  const [drag, setDrag] = useState<Drag | null>(null)
  /** Смещение перетаскиваемого элемента за курсором, пиксели. */
  const [carry, setCarry] = useState(0)
  /** Куда встанет сервер (группа + позиция) либо, для групп, позиция вставки. */
  const [target, setTarget] = useState<Target | null>(null)
  const [groupTarget, setGroupTarget] = useState<number | null>(null)

  const listRef = useRef<HTMLDivElement>(null)
  const rowsRef = useRef<Row[]>([])
  const boxesRef = useRef<{ name: string; top: number; height: number }[]>([])
  const dragRef = useRef<Drag | null>(null)
  const targetRef = useRef<Target | null>(null)
  const groupTargetRef = useRef<number | null>(null)
  const pressRef = useRef<Press | null>(null)
  /** Что доанимировать после перестановки списка: элемент и где он был. */
  const flipRef = useRef<{ sel: string; top: number } | null>(null)
  const pointerRef = useRef({ x: 0, y: 0 })
  const rafRef = useRef(0)
  /** После перетаскивания гасим клик, иначе сервер тут же подключается. */
  const movedRef = useRef(false)

  const groups = useMemo(() => {
    // Разбор строки поиска (`tag:`, `env:`, `fav`) живёт в serverFilter — он покрыт тестами.
    const filtered = filterServers(servers, filter)

    const map = new Map<string, ServerConfig[]>()
    // Пустые группы тоже показываем — иначе только что созданная сразу пропадала бы.
    for (const g of groupOrder) map.set(g, [])
    for (const s of filtered) {
      const g = s.group?.trim() || UNGROUPED
      if (!map.has(g)) map.set(g, [])
      map.get(g)!.push(s)
    }

    for (const list of map.values()) {
      list.sort((a, b) => (a.order ?? 0) - (b.order ?? 0) || a.name.localeCompare(b.name))
    }

    const known = groupOrder.filter((g) => map.has(g))
    const rest = [...map.keys()]
      .filter((g) => g !== UNGROUPED && !groupOrder.includes(g))
      .sort((a, b) => a.localeCompare(b))
    const ordered = [...known, ...rest]
    // «Без группы» всегда внизу, как несортируемый остаток.
    if (map.has(UNGROUPED)) ordered.push(UNGROUPED)
    return ordered.map((g) => [g, map.get(g) ?? []] as const)
  }, [servers, filter, groupOrder])

  const groupsRef = useRef(groups)
  groupsRef.current = groups

  /** Идентификаторы в том порядке, в каком строки сейчас нарисованы. */
  const visibleIds = useMemo(
    () => groups.flatMap(([, items]) => items.map((s) => s.id)),
    [groups]
  )

  // Выделение чистим от исчезнувшего: сервер удалили или фильтр его спрятал. Иначе
  // групповое действие тихо применилось бы к тому, чего пользователь уже не видит.
  useEffect(() => {
    setSel((prev) => pruneSelection(prev, visibleIds))
  }, [visibleIds])

  // Высота строки сервера — из неё берётся размер подсказки «перетащите сюда».
  // Считаем по живому списку: она зависит от плотности интерфейса в настройках.
  useLayoutEffect(() => {
    const list = listRef.current
    const row = list?.querySelector<HTMLElement>('[data-kind="server"]')
    if (!list || !row) return
    const h = row.getBoundingClientRect().height
    if (h > 0) list.style.setProperty('--row-h', `${Math.round(h)}px`)
  }, [servers, collapsed, filter])

  // ——— Перетаскивание ————————————————————————————————————————————————
  //
  // Порядок в DOM во время перетаскивания не меняется: список переставляется
  // один раз, при отпускании. Пока тащим — двигаем только картинку через
  // `transform`. Перестройка DOM на каждое движение мыши накладывает анимации
  // друг на друга, и элементы «размазывает» по экрану.
  //
  // Сам элемент идёт ровно за курсором и поднят над списком (z-index + тень),
  // а все строки между старым и новым местом расступаются на его высоту.
  // Считаем в пикселях по измеренной геометрии, а не «шагами»: заголовки групп
  // ниже строк серверов, и шагами через границу группы промахиваешься.

  /** Позиция вставки в плоском списке строк: перед какой строкой встанет элемент. */
  const insertPos = useCallback((t: Target): number => {
    const rows = rowsRef.current
    const exact = rows.findIndex(
      (r) => r.kind === 'server' && r.group === t.group && r.index === t.index
    )
    if (exact >= 0) return exact
    // В пустой группе щель открываем на месте надписи «перетащите сюда»,
    // а не под ней — иначе непонятно, куда именно попадёт сервер.
    const empty = rows.findIndex((r) => r.kind === 'empty' && r.group === t.group)
    if (empty >= 0) return empty
    // В конец группы или в свёрнутую группу — сразу за её последней строкой.
    let last = -1
    rows.forEach((r, i) => {
      if (r.group === t.group) last = i
    })
    return last + 1
  }, [])

  /** На сколько сдвинуть строку `pos`, чтобы освободить место перетаскиваемому. */
  const rowShift = (key: string): number => {
    if (!drag || drag.kind !== 'server' || !target) return 0
    const rows = rowsRef.current
    const pos = rows.findIndex((r) => r.key === key)
    const from = rows.findIndex((r) => r.key === `s:${drag.id}`)
    if (pos < 0 || from < 0) return 0
    const to = insertPos(target)
    if (to > from) return pos > from && pos < to ? -drag.step : 0
    return pos >= to && pos < from ? drag.step : 0
  }

  /** То же для целых групп: сдвигаются на высоту перетаскиваемой группы. */
  const groupShift = (index: number): number => {
    if (!drag || drag.kind !== 'group' || groupTarget === null) return 0
    const from = drag.index
    const to = groupTarget
    if (to > from) return index > from && index < to ? -drag.step : 0
    return index >= to && index < from ? drag.step : 0
  }

  /** Пересчёт цели по вертикали курсора. */
  const updateTarget = useCallback((clientY: number) => {
    const list = listRef.current
    const d = dragRef.current
    if (!list || !d) return
    const y = clientY - list.getBoundingClientRect().top + list.scrollTop

    if (d.kind === 'group') {
      const boxes = boxesRef.current.filter((b) => b.name !== UNGROUPED)
      let idx = boxes.length
      for (let i = 0; i < boxes.length; i++) {
        if (y < boxes[i].top + boxes[i].height / 2) {
          idx = i
          break
        }
      }
      if (groupTargetRef.current !== idx) {
        groupTargetRef.current = idx
        setGroupTarget(idx)
      }
      return
    }

    const rows = rowsRef.current
    if (!rows.length) return
    // Ближайшая строка, а не «строка под курсором»: между группами есть отступы,
    // и в них курсор не попадал бы ни в одну строку.
    let best = rows[0]
    let bestDist = Infinity
    for (const r of rows) {
      const dist = y < r.top ? r.top - y : y > r.top + r.height ? y - r.top - r.height : 0
      if (dist < bestDist) {
        bestDist = dist
        best = r
      }
    }
    const next: Target =
      best.kind === 'server'
        ? { group: best.group, index: y < best.top + best.height / 2 ? best.index : best.index + 1 }
        : { group: best.group, index: 0 }
    const cur = targetRef.current
    if (cur && cur.group === next.group && cur.index === next.index) return
    targetRef.current = next
    setTarget(next)
  }, [])

  /** Элемент идёт за курсором; за края списка не выпускаем. */
  const updateCarry = useCallback((clientY: number) => {
    const list = listRef.current
    const d = dragRef.current
    if (!list || !d) return
    const lr = list.getBoundingClientRect()
    // Список мог прокрутиться — исходная позиция элемента уехала вместе с ним.
    const top = d.top + lr.top - list.scrollTop
    const want = clientY - d.grabY - top
    const min = lr.top - top
    const max = lr.bottom - d.height - top
    // Группа может быть выше видимой части списка — тогда зажимать некуда.
    setCarry(max < min ? want : Math.max(min, Math.min(want, max)))
  }, [])

  /** Полностью снять перетаскивание: строки возвращаются к обычной вёрстке. */
  const stopDrag = useCallback(() => {
    cancelAnimationFrame(rafRef.current)
    document.body.classList.remove('row-dragging')
    dragRef.current = null
    targetRef.current = null
    groupTargetRef.current = null
    pressRef.current = null
    setDrag(null)
    setTarget(null)
    setGroupTarget(null)
    setCarry(0)
  }, [])

  const beginDrag = useCallback(
    (p: Press, clientY: number) => {
      const list = listRef.current
      if (!list) return
      // Хвосты прошлой анимации приземления: с ними и мерить нечего (геометрия
      // сдвинута), и элемент тащился бы за курсором с задержкой.
      list.querySelectorAll<HTMLElement>('[data-row], [data-groupbox]').forEach((el) => {
        el.style.transition = ''
        el.style.transform = ''
      })
      const lr = list.getBoundingClientRect()
      const base = -lr.top + list.scrollTop

      const rows: Row[] = []
      list.querySelectorAll<HTMLElement>('[data-row]').forEach((el) => {
        const r = el.getBoundingClientRect()
        rows.push({
          key: el.dataset.row!,
          kind: el.dataset.kind as Row['kind'],
          group: el.dataset.group ?? '',
          index: Number(el.dataset.index ?? -1),
          top: r.top + base,
          height: r.height
        })
      })
      rowsRef.current = rows

      const boxes: { name: string; top: number; height: number }[] = []
      list.querySelectorAll<HTMLElement>('[data-groupbox]').forEach((el) => {
        const r = el.getBoundingClientRect()
        boxes.push({ name: el.dataset.groupbox!, top: r.top + base, height: r.height })
      })
      boxesRef.current = boxes

      // У группы двигается весь блок, у сервера — его строка.
      const own =
        p.kind === 'group'
          ? boxes.find((b) => b.name === p.id)
          : rows.find((r) => r.key === `s:${p.id}`)
      if (!own) return

      const d: Drag = {
        kind: p.kind,
        id: p.id,
        group: p.group,
        index: p.index,
        step: own.height,
        height: own.height,
        top: own.top,
        // Курсор держит элемент за ту же точку, за какую взяли: `own.top - base`
        // — это экранная координата верха элемента на момент захвата.
        grabY: p.y - (own.top - base)
      }
      dragRef.current = d
      movedRef.current = true
      document.body.classList.add('row-dragging')
      setDrag(d)
      updateCarry(clientY)
      updateTarget(clientY)

      const tick = (): void => {
        const l = listRef.current
        if (!l || !dragRef.current) return
        const r = l.getBoundingClientRect()
        const y = pointerRef.current.y
        let dy = 0
        if (y < r.top + EDGE) dy = -Math.min(16, (r.top + EDGE - y) / 3)
        else if (y > r.bottom - EDGE) dy = Math.min(16, (y - (r.bottom - EDGE)) / 3)
        if (dy) {
          const before = l.scrollTop
          l.scrollTop += dy
          if (l.scrollTop !== before) {
            updateCarry(y)
            updateTarget(y)
          }
        }
        rafRef.current = requestAnimationFrame(tick)
      }
      rafRef.current = requestAnimationFrame(tick)
    },
    [updateCarry, updateTarget]
  )

  const drop = useCallback(() => {
    const d = dragRef.current
    if (!d) {
      pressRef.current = null
      return
    }
    // Где элемент был в момент броска. Список перестроится синхронно, а оттуда
    // мы доведём его до нового места анимацией — иначе он телепортируется.
    const sel =
      d.kind === 'server'
        ? `[data-row="s:${d.id}"]`
        : `[data-groupbox="${CSS.escape(d.id)}"]`
    const el = listRef.current?.querySelector<HTMLElement>(sel)
    if (el) flipRef.current = { sel, top: el.getBoundingClientRect().top }

    if (d.kind === 'server') {
      const t = targetRef.current
      if (t) {
        // В App перестановка считается по списку БЕЗ переносимого сервера,
        // поэтому при движении вниз внутри группы индекс на единицу меньше.
        const same = t.group === d.group
        const at = same && t.index > d.index ? t.index - 1 : t.index
        if (!same || at !== d.index) onDropServer(d.id, t.group, at)
      }
    } else {
      const to = groupTargetRef.current
      const names = groupsRef.current.map(([g]) => g).filter((g) => g !== UNGROUPED)
      if (to !== null && to !== d.index && to !== d.index + 1) {
        const next = [...names]
        next.splice(d.index, 1)
        next.splice(to > d.index ? to - 1 : to, 0, d.id)
        onDropGroup(next)
      }
    }
    stopDrag()
  }, [onDropServer, onDropGroup, stopDrag])

  useEffect(() => {
    const onMove = (e: PointerEvent): void => {
      pointerRef.current = { x: e.clientX, y: e.clientY }
      const p = pressRef.current
      if (p && !dragRef.current) {
        if (
          Math.abs(e.clientX - p.x) < DRAG_THRESHOLD &&
          Math.abs(e.clientY - p.y) < DRAG_THRESHOLD
        )
          return
        beginDrag(p, e.clientY)
        return
      }
      if (!dragRef.current) return
      e.preventDefault()
      updateCarry(e.clientY)
      updateTarget(e.clientY)
    }
    const onUp = (): void => {
      if (dragRef.current) drop()
      else pressRef.current = null
    }
    const onCancel = (): void => {
      pressRef.current = null
      if (dragRef.current) stopDrag()
    }
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape' && dragRef.current) stopDrag()
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
      window.removeEventListener('keydown', onKey)
    }
  }, [beginDrag, updateCarry, updateTarget, drop, stopDrag])

  useEffect(() => () => document.body.classList.remove('row-dragging'), [])

  // Список уже перестроен, а элемент только что был в другом месте: ставим его
  // обратно нулевым кадром и отпускаем — так он доезжает до нового места, а не
  // прыгает туда. Соседи уже стоят там, где надо: их сдвиг во время перетаскивания
  // равен настоящему изменению вёрстки, поэтому их трогать не нужно.
  useLayoutEffect(() => {
    const f = flipRef.current
    if (!f) return
    flipRef.current = null
    const el = listRef.current?.querySelector<HTMLElement>(f.sel)
    if (!el) return
    const delta = f.top - el.getBoundingClientRect().top
    if (Math.abs(delta) < 1) return
    el.style.transition = 'none'
    el.style.transform = `translateY(${delta}px)`
    // Принудительный пересчёт: без него браузер объединит оба присваивания
    // в одно и анимации не будет.
    void el.offsetHeight
    el.style.transition = 'transform 170ms cubic-bezier(0.2, 0.7, 0.3, 1)'
    el.style.transform = ''
    const done = (): void => {
      el.style.transition = ''
      el.style.transform = ''
      el.removeEventListener('transitionend', done)
    }
    el.addEventListener('transitionend', done)
  })

  const press = (e: React.PointerEvent, p: Omit<Press, 'el' | 'x' | 'y'>): void => {
    // Левая кнопка и не по кнопкам-иконкам внутри строки.
    if (e.button !== 0) return
    if ((e.target as HTMLElement).closest('button')) return
    movedRef.current = false
    pressRef.current = { ...p, el: e.currentTarget as HTMLElement, x: e.clientX, y: e.clientY }
  }

  const emptyList = groups.every(([, items]) => items.length === 0)

  const openPanelMenu = (e: React.MouseEvent): void => {
    e.preventDefault()
    e.stopPropagation()
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: 'Новый сервер', onClick: onNew },
        { label: 'Новая группа', onClick: onNewGroup },
        { label: 'Выполнить на нескольких…', onClick: onMultiExec, separated: true },
        { label: 'Настройки групп', onClick: onOpenGroups }
      ]
    })
  }

  const openServerMenu = (e: React.MouseEvent, s: ServerConfig): void => {
    e.preventDefault()
    e.stopPropagation()
    // Действие применяется ко всему выделению, только если щёлкнули внутри него.
    // Правый клик по чужой строке — это работа с ней одной, а не с прошлым выбором.
    const ids = targetsFor(sel, s.id)
    const many = ids.length > 1
    const suffix = many ? ` (${ids.length})` : ''
    const forEach = (fn: (id: string) => void): (() => void) => () => ids.forEach(fn)
    const moveItems: MenuItem[] = groupOrder
      .filter((g) => many || g !== (s.group?.trim() || UNGROUPED))
      .map((g) => ({
        label: `Перенести в «${g}»${suffix}`,
        onClick: forEach((id) => onDropServer(id, g))
      }))
    // Метка среды переключается прямо из списка: она нужна раньше, чем кто-то полезет
    // в форму — чтобы «прод» было видно до того, как в него что-нибудь выполнят.
    const envItems: MenuItem[] = onPatch
      ? [
          ...ENVS.filter((e2) => many || e2 !== s.env).map((e2) => ({
            label: `Пометить как ${ENV_LABEL[e2]}${suffix}`,
            onClick: forEach((id) => onPatch(id, { env: e2 }))
          })),
          ...(many || s.env
            ? [
                {
                  label: `Снять метку среды${suffix}`,
                  onClick: forEach((id) => onPatch(id, { env: undefined }))
                }
              ]
            : [])
        ]
      : []
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        {
          label: many ? `Подключиться ко всем${suffix}` : 'Подключиться',
          onClick: () => ids.forEach((id) => {
            const cfg = servers.find((x) => x.id === id)
            if (cfg) onConnect(cfg)
          })
        },
        ...(many ? [] : [{ label: 'Изменить', onClick: () => onEdit(s) }]),
        ...(onPatch
          ? [
              {
                label: (many || !s.favorite ? 'В избранное' : 'Убрать из избранного') + suffix,
                onClick: forEach((id) => onPatch(id, { favorite: many ? true : !s.favorite }))
              },
              ...(many
                ? [{ label: `Убрать из избранного${suffix}`, onClick: forEach((id) => onPatch(id, { favorite: undefined })) }]
                : []),
              ...envItems
            ]
          : []),
        ...moveItems,
        ...(s.group
          ? [{ label: 'Убрать из группы', onClick: () => onDropServer(s.id, UNGROUPED) }]
          : []),
        {
          label: many ? `Удалить${suffix}` : 'Удалить',
          danger: true,
          separated: true,
          onClick: () => {
            const question = many
              ? `Удалить серверы (${ids.length})? Действие необратимо.`
              : `Удалить сервер «${s.name}»?`
            if (confirm(question)) ids.forEach(onDelete)
          }
        }
      ]
    })
  }

  const openGroupMenu = (e: React.MouseEvent, group: string): void => {
    e.preventDefault()
    e.stopPropagation()
    const isUngrouped = group === UNGROUPED
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: 'Новый сервер', onClick: onNew },
        { label: 'Новая группа', onClick: onNewGroup },
        ...(isUngrouped
          ? []
          : [{ label: `Свернуть/развернуть «${group}»`, onClick: () => onToggleGroup(group) }]),
        { label: 'Настройки групп', onClick: onOpenGroups, separated: true }
      ]
    })
  }

  const draggableGroups = groups.filter(([g]) => g !== UNGROUPED).length

  return (
    <aside
      className={'sidebar' + (collapsed ? ' collapsed' : '')}
      style={{ width: collapsed ? 56 : width }}
    >
      <div className="sidebar-header">
        {collapsed ? (
          <button className="icon-btn" title="Развернуть список серверов" onClick={onToggleCollapse}>
            <Icon name="chevron-right" />
          </button>
        ) : (
          <>
            <span className="logo">
              <Icon name="logo" size={16} /> Serein
            </span>
            <div className="sidebar-header-actions">
              <button className="icon-btn" title="Свернуть список серверов" onClick={onToggleCollapse}>
                <Icon name="chevron-left" />
              </button>
              <button
                className="icon-btn"
                title="Импортировать серверы"
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect()
                  setMenu({
                    x: r.left,
                    y: r.bottom + 6,
                    items: [
                      { label: 'Из ~/.ssh/config', onClick: () => onImport('ssh') },
                      ...(showPuttyImport
                        ? [
                            { label: 'Из сессий PuTTY', onClick: () => onImport('putty') },
                            { label: 'Из MobaXterm', onClick: () => onImport('mobaxterm') },
                            { label: 'Из XShell', onClick: () => onImport('xshell') },
                            { label: 'Из SecureCRT', onClick: () => onImport('securecrt') }
                          ]
                        : [])
                    ]
                  })
                }}
              >
                <Icon name="import" />
              </button>
              <button className="icon-btn" title="Добавить сервер" onClick={onNew}>
                <Icon name="plus" />
              </button>
            </div>
          </>
        )}
      </div>

      {!collapsed && (
        <input
          className="search"
          placeholder="Поиск серверов…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      )}

      <div
        ref={listRef}
        className="server-list"
        onContextMenu={collapsed ? undefined : openPanelMenu}
      >
        {/*
          Пустой список и пустая выдача фильтра — разные вещи. Раньше на «env:prod» без
          подходящих серверов выводилось «Серверов пока нет», и выглядело это так, будто
          профили пропали.
        */}
        {!collapsed && emptyList && filter.trim() && (
          <div className="hint">
            Ничего не найдено. В поиске работают «tag:web», «env:prod» и «fav».
          </div>
        )}
        {!collapsed && emptyList && !filter.trim() && groupOrder.length === 0 && (
          <div className="hint">Серверов пока нет. Нажмите «+» или кликните правой кнопкой.</div>
        )}

        {groups.map(([group, items], gi) => {
          const isUngrouped = group === UNGROUPED
          // В свёрнутой панели заголовков групп нет — если оставить скрытие,
          // список окажется пустым и до серверов не добраться.
          const folded = !collapsed && collapsedGroups.includes(group)
          const dragged = drag?.kind === 'group' && drag.id === group
          const shift = dragged ? carry : groupShift(gi)
          // Подсветка нужна там, где щели не видно: чужая группа, свёрнутая, пустая.
          const highlight =
            drag?.kind === 'server' && target?.group === group && target.group !== drag.group
          return (
            <div
              key={group || '__ungrouped__'}
              data-groupbox={group}
              className={
                'group' +
                (highlight ? ' drop-target' : '') +
                (dragged ? ' row-drag' : '') +
                (drag?.kind === 'group' && !dragged ? ' row-move' : '')
              }
              style={drag?.kind === 'group' ? { transform: `translateY(${shift}px)` } : undefined}
            >
              {!collapsed && (
                <div
                  data-row={`h:${group}`}
                  data-kind="header"
                  data-group={group}
                  className={
                    'group-title' +
                    (drag?.kind === 'server' ? ' row-move' : '') +
                    (!isUngrouped && draggableGroups > 1 ? ' grabbable' : '')
                  }
                  style={
                    drag?.kind === 'server'
                      ? { transform: `translateY(${rowShift(`h:${group}`)}px)` }
                      : undefined
                  }
                  onPointerDown={(e) => {
                    if (isUngrouped || draggableGroups < 2) return
                    press(e, { kind: 'group', id: group, group, index: gi })
                  }}
                  onClick={() => {
                    if (movedRef.current) return
                    onToggleGroup(group)
                  }}
                  onContextMenu={(e) => openGroupMenu(e, group)}
                  title={folded ? 'Развернуть' : 'Свернуть'}
                >
                  <Icon name={folded ? 'chevron-right' : 'chevron-down'} size={12} />
                  <span className="group-title-name">{isUngrouped ? UNGROUPED_TITLE : group}</span>
                  <span className="group-title-count">{items.length}</span>
                </div>
              )}

              {!folded &&
                items.map((s, idx) => {
                  const self = drag?.kind === 'server' && drag.id === s.id
                  const dy = self ? carry : drag?.kind === 'server' ? rowShift(`s:${s.id}`) : 0
                  return (
                    <div
                      key={s.id}
                      data-row={`s:${s.id}`}
                      data-kind="server"
                      data-group={group}
                      data-index={idx}
                      className={
                        'server-item' +
                        (isSelected(sel, s.id) ? ' selected' : '') +
                        (self ? ' row-drag' : '') +
                        (drag?.kind === 'server' && !self ? ' row-move' : '')
                      }
                      style={
                        drag?.kind === 'server' ? { transform: `translateY(${dy}px)` } : undefined
                      }
                      onPointerDown={(e) =>
                        press(e, { kind: 'server', id: s.id, group, index: idx })
                      }
                      onClick={(e) => {
                        if (movedRef.current) return
                        // Клик по кнопке строки (звезда, подключение, правка) — это её
                        // действие, а не работа со списком: выделение он менять не должен.
                        if ((e.target as HTMLElement).closest('button')) return
                        if (collapsed) {
                          onConnect(s)
                          return
                        }
                        setSel((prev) =>
                          clickSelect(prev, s.id, visibleIds, {
                            ctrl: e.ctrlKey || e.metaKey,
                            shift: e.shiftKey
                          })
                        )
                      }}
                      onDoubleClick={() => {
                        if (!movedRef.current) onConnect(s)
                      }}
                      onContextMenu={(e) => openServerMenu(e, s)}
                      title={serverTooltip(s)}
                    >
                      <span
                        className="dot-wrap"
                        title={statuses?.[s.id] ? `Статус: ${statuses[s.id]}` : undefined}
                      >
                        <span className="dot" style={{ background: s.color || '#7aa2f7' }} />
                        {statuses?.[s.id] && (
                          <span
                            className={
                              'dot-status' +
                              (statuses[s.id] === 'connecting' || statuses[s.id] === 'reconnecting'
                                ? ' pulse'
                                : '')
                            }
                            style={{ background: STATUS_DOT[statuses[s.id]] }}
                          />
                        )}
                      </span>
                      {!collapsed && (
                        <>
                          <div className="server-info">
                            <div className="server-name">
                              {s.favorite && (
                                <span className="fav-mark" title="В избранном">
                                  ★
                                </span>
                              )}
                              <span className="server-name-text">{s.name}</span>
                              {s.env && (
                                <span className={`env-badge env-${s.env}`} title={`Среда: ${ENV_LABEL[s.env]}`}>
                                  {ENV_LABEL[s.env]}
                                </span>
                              )}
                            </div>
                            <div className="server-host">{serverSubtitle(s)}</div>
                          </div>
                          <div className="server-actions">
                            {onPatch && (
                              <button
                                className={'mini' + (s.favorite ? ' fav-on' : '')}
                                title={s.favorite ? 'Убрать из избранного' : 'В избранное'}
                                onClick={() => onPatch(s.id, { favorite: !s.favorite })}
                              >
                                <Icon name="star" size={14} />
                              </button>
                            )}
                            <button className="mini" title="Подключиться" onClick={() => onConnect(s)}>
                              <Icon name="play" size={14} />
                            </button>
                            <button className="mini" title="Изменить" onClick={() => onEdit(s)}>
                              <Icon name="edit" size={14} />
                            </button>
                            <button
                              className="mini danger"
                              title="Удалить"
                              onClick={() => {
                                if (confirm(`Удалить сервер «${s.name}»?`)) onDelete(s.id)
                              }}
                            >
                              <Icon name="trash" size={14} />
                            </button>
                          </div>
                        </>
                      )}
                    </div>
                  )
                })}

              {!collapsed && !folded && items.length === 0 && !isUngrouped && (
                <div
                  data-row={`e:${group}`}
                  data-kind="empty"
                  data-group={group}
                  className={'group-empty' + (drag?.kind === 'server' ? ' row-move' : '')}
                  style={
                    drag?.kind === 'server'
                      ? { transform: `translateY(${rowShift(`e:${group}`)}px)` }
                      : undefined
                  }
                >
                  Перетащите сюда сервер
                </div>
              )}

              {/* В узкой панели заголовков нет — группы разделяем чертой. */}
              {collapsed && items.length > 0 && <div className="group-divider" />}
            </div>
          )
        })}
      </div>

      <div className="sidebar-footer">
        {collapsed ? (
          <>
            <button className="icon-btn" title="Локальный терминал" onClick={onOpenLocal}>
              <Icon name="desktop" />
            </button>
            <button className="icon-btn" title="Генерация ключей" onClick={onOpenKeyGen}>
              <Icon name="key" />
            </button>
            <button className="icon-btn" title="Настройки" onClick={onOpenSettings}>
              <Icon name="settings" />
            </button>
          </>
        ) : (
          <>
            <button className="full-btn" onClick={onOpenLocal}>
              <Icon name="desktop" /> Локальный терминал
            </button>
            <button className="full-btn" onClick={onOpenKeyGen}>
              <Icon name="key" /> Генерация ключей
            </button>
            <button className="full-btn" onClick={onOpenSettings}>
              <Icon name="settings" /> Настройки
            </button>
          </>
        )}
      </div>

      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </aside>
  )
}
