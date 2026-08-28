import { Icon } from './Icon'

export function AuxReattachButton({ onClick }: { onClick: () => void | Promise<void> }): JSX.Element {
  return (
    <button type="button" className="mini aux-reattach-btn" title="Вернуть в главное окно" onClick={() => void onClick()}>
      <Icon name="back" size={14} />
    </button>
  )
}