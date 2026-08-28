/** Sessions moved to detached window — do not close on main unmount. */
const detached = new Set<string>()

export function markSessionDetached(id: string): void {
  detached.add(id)
}

export function clearDetachedMark(id: string): void {
  detached.delete(id)
}

export function shouldPreserveSession(id: string): boolean {
  return detached.has(id)
}
