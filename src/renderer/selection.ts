/**
 * Выделение нескольких серверов в списке.
 *
 * Правила выделения — те же, к которым приучил проводник: обычный клик выделяет один,
 * Ctrl добавляет и убирает по одному, Shift берёт отрезок от последнего выделенного.
 * Логика вынесена сюда, потому что «якорь для Shift» — единственная её нетривиальная
 * часть, и ошибиться в нём легко: без якоря отрезок каждый раз считается от края и
 * выделяет не то, что ждёт пользователь.
 */

export interface Selection {
  /** Выбранные идентификаторы. */
  ids: string[]
  /** От какой строки отсчитывается отрезок при Shift. */
  anchor?: string
}

export const EMPTY_SELECTION: Selection = { ids: [] }

export interface ClickMods {
  ctrl?: boolean
  shift?: boolean
}

/**
 * Новое выделение после клика по строке `id`.
 *
 * `order` — идентификаторы в том порядке, в каком они сейчас нарисованы: отрезок Shift
 * считается по видимому порядку, а не по порядку хранения. Если список отфильтрован,
 * пользователь ждёт отрезок именно из того, что перед глазами.
 */
export function clickSelect(sel: Selection, id: string, order: string[], mods: ClickMods = {}): Selection {
  if (mods.shift && sel.anchor && order.includes(sel.anchor)) {
    const from = order.indexOf(sel.anchor)
    const to = order.indexOf(id)
    if (to === -1) return sel
    const [a, b] = from <= to ? [from, to] : [to, from]
    // Якорь сохраняем: протягивая Shift туда-сюда, пользователь ждёт одну точку отсчёта.
    return { ids: order.slice(a, b + 1), anchor: sel.anchor }
  }
  if (mods.ctrl) {
    const has = sel.ids.includes(id)
    const ids = has ? sel.ids.filter((x) => x !== id) : [...sel.ids, id]
    // Снятая строка перестаёт быть якорем — иначе Shift потянется от того, чего нет.
    return { ids, anchor: has ? undefined : id }
  }
  return { ids: [id], anchor: id }
}

/** Убирает из выделения то, чего больше нет в списке (удалили сервер, сузили фильтр). */
export function pruneSelection(sel: Selection, present: string[]): Selection {
  const ids = sel.ids.filter((id) => present.includes(id))
  if (ids.length === sel.ids.length && (!sel.anchor || present.includes(sel.anchor))) return sel
  return { ids, anchor: sel.anchor && present.includes(sel.anchor) ? sel.anchor : undefined }
}

export function isSelected(sel: Selection, id: string): boolean {
  return sel.ids.includes(id)
}

/**
 * Выделение, на которое действуют групповые операции.
 *
 * Пока выбрана одна строка (или не выбрано ничего), действие относится к тому серверу, по
 * которому щёлкнули — иначе меню «удалить 1 сервер» вместо «удалить этот» только путает.
 */
export function targetsFor(sel: Selection, clicked: string): string[] {
  return sel.ids.length > 1 && sel.ids.includes(clicked) ? sel.ids : [clicked]
}
