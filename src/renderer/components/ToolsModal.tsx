import { useState } from 'react'
import { errText } from '../errText'

type Tab = 'port' | 'scan' | 'dns' | 'tls' | 'subnet' | 'hash' | 'jwt'

interface ConnectedSession {
  sessionId: string
  title: string
}

/**
 * Откуда выполнять проверку: со своей машины или глазами одного из серверов.
 *
 * Различие не косметическое. «Я не вижу этот адрес» и «его не видит сервер» — разные
 * новости, и при разборе неполадки нужна почти всегда вторая: у сервера свои маршруты,
 * свой DNS и свой `/etc/hosts`. Утилита, которая умеет только первое, отвечает не на тот
 * вопрос, который ей задают.
 */
const HERE = 'here'

interface Props {
  /** Живые SSH-сессии: только через них можно спросить сервер. */
  connectedSessions: ConnectedSession[]
  /**
   * С чего начать выбор «откуда».
   *
   * У окна две двери. Из общего меню человек ещё не выбрал сервер — начинаем со своей
   * машины. Из рельсы сервера он его уже выбрал, открыв вкладку, и спрашивать второй раз
   * незачем: подставляем этот сервер.
   */
  defaultFrom?: string
  onClose: () => void
}

const TABS: { id: Tab; label: string }[] = [
  { id: 'port', label: 'Порт' },
  { id: 'scan', label: 'Диапазон' },
  { id: 'dns', label: 'DNS' },
  { id: 'tls', label: 'TLS' },
  { id: 'subnet', label: 'Подсеть' },
  { id: 'hash', label: 'Хеш' },
  { id: 'jwt', label: 'JWT' }
]

function JsonOut({ value }: { value: unknown }): JSX.Element {
  return (
    <pre className="tools-out">{typeof value === 'string' ? value : JSON.stringify(value, null, 2)}</pre>
  )
}

/** Выбор «откуда». Показывается только там, где вопрос вообще имеет два ответа. */
function From({
  value,
  onChange,
  sessions
}: {
  value: string
  onChange: (v: string) => void
  sessions: ConnectedSession[]
}): JSX.Element {
  return (
    <label className="tools-from">
      Откуда
      <select value={value} onChange={(e) => onChange(e.target.value)}>
        <option value={HERE}>с этой машины</option>
        {sessions.map((s) => (
          <option key={s.sessionId} value={s.sessionId}>
            с сервера: {s.title}
          </option>
        ))}
      </select>
      {sessions.length === 0 && (
        <span className="hint">Подключитесь к серверу, чтобы спросить и его тоже</span>
      )}
    </label>
  )
}

export function ToolsModal({ connectedSessions, defaultFrom, onClose }: Props): JSX.Element {
  const [tab, setTab] = useState<Tab>('port')
  // Одна на обе вкладки: человек обычно разбирается с одним сервером за раз.
  // Сессия могла отвалиться, пока окно было закрыто, — тогда возвращаемся к своей машине,
  // иначе выбор указывал бы на то, чего уже нет.
  const [from, setFrom] = useState<string>(() =>
    defaultFrom && connectedSessions.some((s) => s.sessionId === defaultFrom) ? defaultFrom : HERE
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [out, setOut] = useState<unknown>(null)

  const [portHost, setPortHost] = useState('127.0.0.1')
  const [portNum, setPortNum] = useState('22')
  const [scanHost, setScanHost] = useState('127.0.0.1')
  const [scanFrom, setScanFrom] = useState('1')
  const [scanTo, setScanTo] = useState('1024')
  const [dnsName, setDnsName] = useState('example.com')
  const [tlsHost, setTlsHost] = useState('nehade.xyz')
  const [tlsPort, setTlsPort] = useState('443')
  const [subnetIn, setSubnetIn] = useState('192.168.0.0/24')
  const [hashAlgo, setHashAlgo] = useState('sha256')
  const [hashText, setHashText] = useState('')
  const [jwtToken, setJwtToken] = useState('')

  const run = async (fn: () => Promise<unknown>): Promise<void> => {
    setBusy(true)
    setError(null)
    setOut(null)
    try {
      setOut(await fn())
    } catch (e) {
      setError(errText(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal tools-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Утилиты</h2>
        <div className="tools-tabs">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              className={'mini' + (tab === t.id ? ' on' : '')}
              onClick={() => {
                setTab(t.id)
                setError(null)
                setOut(null)
              }}
            >
              {t.label}
            </button>
          ))}
        </div>

        {tab === 'port' && (
          <div className="tools-pane">
            <div className="row">
              <label style={{ flex: 2 }}>
                Хост
                <input value={portHost} onChange={(e) => setPortHost(e.target.value)} placeholder="host или host:port" />
              </label>
              <label style={{ flex: 1 }}>
                Порт
                <input value={portNum} onChange={(e) => setPortNum(e.target.value)} type="number" min={1} max={65535} />
              </label>
            </div>
            <From value={from} onChange={setFrom} sessions={connectedSessions} />
            <button
              className="primary"
              disabled={busy}
              onClick={() =>
                void run(() =>
                  from === HERE
                    ? window.api.tools.portTest(portHost, Number(portNum))
                    : window.api.tools.portTestOn(from, portHost, Number(portNum))
                )
              }
            >
              {from === HERE ? 'Проверить TCP' : 'Проверить TCP с сервера'}
            </button>
          </div>
        )}

        {tab === 'scan' && (
          <div className="tools-pane">
            <label>
              Хост
              <input value={scanHost} onChange={(e) => setScanHost(e.target.value)} placeholder="host" />
            </label>
            <div className="row">
              <label style={{ flex: 1 }}>
                С порта
                <input value={scanFrom} onChange={(e) => setScanFrom(e.target.value)} type="number" min={1} max={65535} />
              </label>
              <label style={{ flex: 1 }}>
                По порт
                <input value={scanTo} onChange={(e) => setScanTo(e.target.value)} type="number" min={1} max={65535} />
              </label>
            </div>
            <From value={from} onChange={setFrom} sessions={connectedSessions} />
            <p className="hint">
              За раз — не больше 1024 портов. С сервера проверки идут по очереди, поэтому там
              диапазон лучше держать узким: сотня закрытых портов — это около полутора минут.
            </p>
            <button
              className="primary"
              disabled={busy}
              onClick={() =>
                void run(() =>
                  from === HERE
                    ? window.api.tools.portScan(scanHost, Number(scanFrom), Number(scanTo))
                    : window.api.tools.portScanOn(from, scanHost, Number(scanFrom), Number(scanTo))
                )
              }
            >
              {busy ? 'Смотрю…' : from === HERE ? 'Просмотреть' : 'Просмотреть с сервера'}
            </button>
          </div>
        )}

        {tab === 'dns' && (
          <div className="tools-pane">
            <label>
              Имя
              <input value={dnsName} onChange={(e) => setDnsName(e.target.value)} placeholder="example.com" />
            </label>
            <From value={from} onChange={setFrom} sessions={connectedSessions} />
            <button
              className="primary"
              disabled={busy}
              onClick={() =>
                void run(() =>
                  from === HERE
                    ? window.api.tools.dnsLookup(dnsName)
                    : window.api.tools.dnsLookupOn(from, dnsName)
                )
              }
            >
              {from === HERE ? 'Разрешить' : 'Разрешить с сервера'}
            </button>
          </div>
        )}

        {tab === 'tls' && (
          <div className="tools-pane">
            <div className="row">
              <label style={{ flex: 2 }}>
                Хост
                <input value={tlsHost} onChange={(e) => setTlsHost(e.target.value)} />
              </label>
              <label style={{ flex: 1 }}>
                Порт
                <input value={tlsPort} onChange={(e) => setTlsPort(e.target.value)} type="number" />
              </label>
            </div>
            <button
              className="primary"
              disabled={busy}
              onClick={() => void run(() => window.api.tools.tlsCert(tlsHost, Number(tlsPort) || 443))}
            >
              Получить сертификат
            </button>
          </div>
        )}

        {tab === 'subnet' && (
          <div className="tools-pane">
            <label>
              CIDR или «IP маска»
              <input value={subnetIn} onChange={(e) => setSubnetIn(e.target.value)} placeholder="10.0.0.0/24" />
            </label>
            <button className="primary" disabled={busy} onClick={() => void run(() => window.api.tools.subnet(subnetIn))}>
              Посчитать
            </button>
          </div>
        )}

        {tab === 'hash' && (
          <div className="tools-pane">
            <label>
              Алгоритм
              <select value={hashAlgo} onChange={(e) => setHashAlgo(e.target.value)}>
                <option value="md5">MD5</option>
                <option value="sha1">SHA-1</option>
                <option value="sha256">SHA-256</option>
                <option value="sha512">SHA-512</option>
              </select>
            </label>
            <label>
              Текст
              <textarea rows={4} value={hashText} onChange={(e) => setHashText(e.target.value)} />
            </label>
            <button className="primary" disabled={busy} onClick={() => void run(() => window.api.tools.hash(hashAlgo, hashText))}>
              Вычислить
            </button>
          </div>
        )}

        {tab === 'jwt' && (
          <div className="tools-pane">
            <label>
              JWT (без проверки подписи)
              <textarea rows={4} value={jwtToken} onChange={(e) => setJwtToken(e.target.value)} placeholder="eyJ..." />
            </label>
            <button className="primary" disabled={busy} onClick={() => void run(() => window.api.tools.jwtDecode(jwtToken))}>
              Разобрать
            </button>
          </div>
        )}

        {error && <p className="tools-error">{error}</p>}
        {out != null && <JsonOut value={out} />}

        <div className="modal-actions">
          <button onClick={onClose}>Закрыть</button>
        </div>
      </div>
    </div>
  )
}
