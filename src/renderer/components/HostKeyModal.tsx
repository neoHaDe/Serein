import type { HostKeyRequest } from '../../shared/types'

interface Props {
  request: HostKeyRequest
  onAnswer: (accept: boolean) => void
}

/**
 * Вопрос о ключе сервера. Два принципиально разных случая:
 * первое подключение (обычное дело) и смена ключа (либо сервер переустановили,
 * либо кто-то встал посередине). Поэтому у них разный тон и разная кнопка по умолчанию.
 */
export function HostKeyModal({ request, onAnswer }: Props): JSX.Element {
  const changed = request.kind === 'changed'

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onAnswer(false)}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{changed ? 'Ключ сервера изменился' : 'Незнакомый сервер'}</h2>

        {changed ? (
          <p className="hostkey-warn">
            Отпечаток ключа <b>{request.host}</b> не совпадает с запомненным. Так бывает, когда
            сервер переустановили или пересоздали ключи — но так же выглядит и попытка
            вклиниться в соединение. Если вы не меняли сервер, отказывайтесь.
          </p>
        ) : (
          <p className="hostkey-hint">
            Вы подключаетесь к <b>{request.host}</b> впервые. Сверьте отпечаток с тем, что
            показывает сам сервер (<code>ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub</code>),
            и запомните его.
          </p>
        )}

        {changed && (
          <label>
            Запомненный ранее
            <input readOnly value={request.previous} className="hostkey-fp old" />
          </label>
        )}

        <label>
          {changed ? 'Новый отпечаток' : 'Отпечаток ключа'}
          <input readOnly value={request.fingerprint} className="hostkey-fp" />
        </label>

        <div className="modal-actions">
          <button className="secondary" onClick={() => onAnswer(false)} autoFocus>
            Отказаться
          </button>
          <button className={changed ? 'danger' : 'primary'} onClick={() => onAnswer(true)}>
            {changed ? 'Всё равно доверять' : 'Доверять и запомнить'}
          </button>
        </div>
      </div>
    </div>
  )
}
