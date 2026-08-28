import { Icon } from './Icon'

export function WsDetachButton({ onClick }: { onClick: () => void | Promise<void> }): JSX.Element {
  return (
    <button className="mini" title="Открепить в отдельное окно" onClick={() => void onClick()}>
      <Icon name="external" size={14} />
    </button>
  )
}
