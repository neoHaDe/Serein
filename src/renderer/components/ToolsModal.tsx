import { lazy, Suspense, useRef, useState } from 'react'
import { errText } from '../errText'
import type { DiffResult } from './DiffView'

/*
 * Вид сравнения грузится по требованию: он тянет разбор языков ради подсветки, а
 * платить за это при запуске должны только те, кто сравнением пользуется.
 */
const DiffView = lazy(() => import('./DiffView').then((m) => ({ default: m.DiffView })))

/** Похож ли ответ на результат сравнения файлов, а не на что-то другое. */
function isDiff(v: unknown): v is DiffResult {
  return (
    typeof v === 'object' &&
    v !== null &&
    'same' in v &&
    'lines' in v &&
    Array.isArray((v as { lines: unknown }).lines)
  )
}
import type { IconName } from './Icon'
import { Icon } from './Icon'
import { RemoteFilePicker } from './RemoteFilePicker'

type Tab =
  | 'port'
  | 'scan'
  | 'trace'
  | 'http'
  | 'dns'
  | 'tls'
  | 'ldap'
  | 'diff'
  | 'subnet'
  | 'hash'
  | 'jwt'

interface ConnectedSession {
  sessionId: string
  title: string
}

/**
 * Откуда выполнять проверку: со своей машины или глазами одного из серверов.
 *
 * Различие не косметическое. «Я не вижу этот адрес» и «его не видит сервер» - разные
 * новости, и при разборе неполадки нужна почти всегда вторая: у сервера свои маршруты,
 * свой DNS и свой `/etc/hosts`. Утилита, которая умеет только первое, отвечает не на тот
 * вопрос, который ей задают.
 */
const HERE = 'here'

interface Props {
  /** Живые SSH-сессии: только через них можно спросить сервер. */
  connectedSessions: ConnectedSession[]
  /**
   * Закрыть панель. Есть только у модального варианта - вкладку закрывают её крестиком
   * в общей полосе, и вторая кнопка там была бы лишней.
   */
  onClose?: () => void
  /**
   * С чего начать выбор «откуда».
   *
   * У окна две двери. Из общего меню человек ещё не выбрал сервер - начинаем со своей
   * машины. Из рельсы сервера он его уже выбрал, открыв вкладку, и спрашивать второй раз
   * незачем: подставляем этот сервер.
   */
  defaultFrom?: string
}

/**
 * Порядок не алфавитный и не случайный: сверху то, что спрашивают у сети, снизу - то,
 * что считается на месте и сервера не касается вовсе. Между ними - черта.
 */
const TABS: { id: Tab; label: string; icon: IconName; hint: string; local?: true }[] = [
  { id: 'port', label: 'Порт', icon: 'link', hint: 'Открыт ли TCP-порт' },
  { id: 'scan', label: 'Диапазон', icon: 'list', hint: 'Какие порты открыты' },
  { id: 'trace', label: 'Маршрут', icon: 'tunnel', hint: 'Через какие узлы идёт трафик' },
  { id: 'http', label: 'HTTP', icon: 'external', hint: 'Что отвечает служба' },
  { id: 'dns', label: 'DNS', icon: 'search', hint: 'В какой адрес разрешается имя' },
  { id: 'tls', label: 'TLS', icon: 'key', hint: 'Чей сертификат и до какого числа' },
  { id: 'ldap', label: 'LDAP', icon: 'server', hint: 'Пускает ли каталог и есть ли запись' },
  { id: 'diff', label: 'Сравнить файлы', icon: 'copy', hint: 'Одинаковы ли и чем отличаются' },
  { id: 'subnet', label: 'Подсеть', icon: 'broadcast', hint: 'Границы сети по маске', local: true },
  { id: 'hash', label: 'Хеш', icon: 'snippets', hint: 'Контрольная сумма текста', local: true },
  { id: 'jwt', label: 'JWT', icon: 'file', hint: 'Что внутри токена', local: true }
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

/**
 * Одна сторона сравнения: где лежит файл и какой.
 *
 * Выбор машины и путь стоят рядом не для красоты - путь без указания машины ничего не
 * значит, а `/etc/nginx/nginx.conf` есть на каждом сервере и везде разный.
 */
function DiffSide({
  label,
  value,
  onChange,
  sessions
}: {
  label: string
  value: { sessionId: string; path: string }
  onChange: (v: { sessionId: string; path: string }) => void
  sessions: ConnectedSession[]
}): JSX.Element {
  const [picking, setPicking] = useState(false)

  // Набранный руками путь - способ ошибиться дважды: опечататься и не заметить, что файла
  // там нет. «Не удалось прочитать» в ответ на это не объясняет ничего.
  const pick = async (): Promise<void> => {
    if (value.sessionId) {
      setPicking(true)
      return
    }
    const chosen = await window.api.tools.pickLocalFile()
    if (chosen) onChange({ ...value, path: chosen })
  }

  return (
    <div className="tools-diff-side">
      <span className="tools-diff-label">{label}</span>
      <div className="row">
        <label style={{ flex: 1 }}>
          Где
          <select
            value={value.sessionId}
            onChange={(e) => {
              // Путь со старой машины на новой почти наверняка не существует, и оставлять
              // его значило бы предлагать заведомо неверное.
              onChange({ sessionId: e.target.value, path: '' })
            }}
          >
            <option value="">на этой машине</option>
            {sessions.map((s) => (
              <option key={s.sessionId} value={s.sessionId}>
                {s.title}
              </option>
            ))}
          </select>
        </label>
        <label style={{ flex: 2 }}>
          Путь
          <input
            value={value.path}
            onChange={(e) => onChange({ ...value, path: e.target.value })}
            placeholder={value.sessionId ? '/etc/nginx/nginx.conf' : 'C:\\путь\\файл.conf'}
          />
        </label>
        <button className="mini picker-open" title="Выбрать файл" onClick={() => void pick()}>
          <Icon name="folder-open" size={14} />
        </button>
      </div>
      {picking && value.sessionId && (
        <RemoteFilePicker
          sessionId={value.sessionId}
          onPick={(p) => {
            onChange({ ...value, path: p })
            setPicking(false)
          }}
          onClose={() => setPicking(false)}
        />
      )}
    </div>
  )
}

export function ToolsModal({ connectedSessions, defaultFrom, onClose }: Props): JSX.Element {
  const [tab, setTab] = useState<Tab>('port')
  // Одна на обе вкладки: человек обычно разбирается с одним сервером за раз.
  // Сессия могла отвалиться, пока окно было закрыто, - тогда возвращаемся к своей машине,
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
  const [traceHost, setTraceHost] = useState('1.1.1.1')
  const [traceHops, setTraceHops] = useState('15')
  const [httpUrl, setHttpUrl] = useState('https://example.com')
  const [httpMethod, setHttpMethod] = useState('GET')
  const [dnsName, setDnsName] = useState('example.com')
  const [tlsHost, setTlsHost] = useState('nehade.xyz')
  const [tlsPort, setTlsPort] = useState('443')
  const [subnetIn, setSubnetIn] = useState('192.168.0.0/24')
  const [hashAlgo, setHashAlgo] = useState('sha256')
  const [hashText, setHashText] = useState('')
  const [jwtToken, setJwtToken] = useState('')
  const [ldapUrl, setLdapUrl] = useState('ldap://127.0.0.1:389')
  const [ldapDn, setLdapDn] = useState('')
  const [ldapPass, setLdapPass] = useState('')
  const [ldapBase, setLdapBase] = useState('')
  const [ldapFilter, setLdapFilter] = useState('(objectClass=*)')
  const [diffA, setDiffA] = useState({ sessionId: '', path: '' })
  const [diffB, setDiffB] = useState({ sessionId: '', path: '' })

  // Номер последнего запуска. Скан диапазона портов на фильтрованном хосте идёт до
  // минуты, а переключаться между утилитами в это время никто не запрещает - и ответ
  // ушедшей утилиты вставал под форму следующей, выглядя как её собственный.
  const runSeqRef = useRef(0)

  const run = async (fn: () => Promise<unknown>): Promise<void> => {
    const seq = ++runSeqRef.current
    setBusy(true)
    setError(null)
    setOut(null)
    try {
      const res = await fn()
      if (seq === runSeqRef.current) setOut(res)
    } catch (e) {
      if (seq === runSeqRef.current) setError(errText(e))
    } finally {
      if (seq === runSeqRef.current) setBusy(false)
    }
  }

  const current = TABS.find((t) => t.id === tab) ?? TABS[0]
  // Первая утилита, которая считается на месте: перед ней в списке ставим разделитель.
  const firstLocal = TABS.find((t) => t.local)?.id

  return (
    <div className="tools-workspace">
      <nav className="ws-rail" aria-label="Утилиты">
        <div className="ws-rail-head">
          <div className="ws-rail-name">
            <Icon name="bolt" size={14} />
            <span className="ws-rail-title">Утилиты</span>
          </div>
          <div className="ws-rail-status">сеть, адреса и расчёты</div>
        </div>
        <div className="ws-nav">
          {TABS.map((t) => (
            <div key={t.id} className={t.id === firstLocal ? 'tools-rail-group' : undefined}>
              {t.id === firstLocal && <span className="tools-rail-divider">Без сервера</span>}
              <button
                type="button"
                className={'ws-nav-item' + (tab === t.id ? ' active' : '')}
                title={t.hint}
                onClick={() => {
                  setTab(t.id)
                  // Ответ прошлой утилиты рядом с формой следующей читался бы как её
                  // собственный - чистим вместе с переключением. Счётчик двигаем здесь
                  // же: он обесценивает ещё летящий ответ, а не только уже пришедший.
                  runSeqRef.current++
                  setBusy(false)
                  setError(null)
                  setOut(null)
                }}
              >
                <Icon name={t.icon} size={14} />
                {t.label}
              </button>
            </div>
          ))}
        </div>
      </nav>

      <div className="ws-panel fill tools-content">
        <div className="ws-head">
          <span className="ws-head-title">
            <Icon name={current.icon} size={15} /> {current.label}
          </span>
          <span className="ws-head-hint">{current.hint}</span>
        </div>
        <div className="tools-body">

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
              За раз - не больше 1024 портов. С сервера проверки идут по очереди, поэтому там
              диапазон лучше держать узким: сотня закрытых портов - это около полутора минут.
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

        {tab === 'trace' && (
          <div className="tools-pane">
            <div className="row">
              <label style={{ flex: 2 }}>
                Хост
                <input value={traceHost} onChange={(e) => setTraceHost(e.target.value)} placeholder="1.1.1.1" />
              </label>
              <label style={{ flex: 1 }}>
                Узлов
                <input value={traceHops} onChange={(e) => setTraceHops(e.target.value)} type="number" min={1} max={30} />
              </label>
            </div>
            <From value={from} onChange={setFrom} sessions={connectedSessions} />
            <p className="hint">
              Маршрут строит системная программа: <code>tracert</code> на Windows,
              <code> traceroute</code> на юниксах. На минимальных серверах её может не быть
              вовсе - тогда это будет сказано прямо, а не показано пустым списком.
            </p>
            <button
              className="primary"
              disabled={busy}
              onClick={() =>
                void run(() =>
                  from === HERE
                    ? window.api.tools.trace(traceHost, Number(traceHops))
                    : window.api.tools.traceOn(from, traceHost, Number(traceHops))
                )
              }
            >
              {busy ? 'Строю маршрут…' : from === HERE ? 'Построить маршрут' : 'Построить с сервера'}
            </button>
          </div>
        )}

        {tab === 'http' && (
          <div className="tools-pane">
            <div className="row">
              <label style={{ flex: 3 }}>
                Адрес
                <input
                  value={httpUrl}
                  onChange={(e) => setHttpUrl(e.target.value)}
                  placeholder="https://example.com/health"
                />
              </label>
              <label style={{ flex: 1 }}>
                Метод
                <select value={httpMethod} onChange={(e) => setHttpMethod(e.target.value)}>
                  <option value="GET">GET</option>
                  <option value="HEAD">HEAD</option>
                </select>
              </label>
            </div>
            <From value={from} onChange={setFrom} sessions={connectedSessions} />
            <p className="hint">
              Со своей машины запрос идёт своими силами, и каждый переход по редиректу виден
              отдельным шагом. С сервера - через <code>curl</code> или <code>wget</code>, и там
              будет только итог: чужими программами цепочку не разложить.
            </p>
            <button
              className="primary"
              disabled={busy}
              onClick={() =>
                void run(() =>
                  from === HERE
                    ? window.api.tools.http(httpUrl, httpMethod)
                    : window.api.tools.httpOn(from, httpUrl, httpMethod)
                )
              }
            >
              {busy ? 'Запрашиваю…' : from === HERE ? 'Запросить' : 'Запросить с сервера'}
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

        {tab === 'ldap' && (
          <div className="tools-pane">
            <label>
              Адрес каталога
              <input value={ldapUrl} onChange={(e) => setLdapUrl(e.target.value)} placeholder="ldap://dc.example.com" />
            </label>
            <div className="row">
              <label style={{ flex: 1 }}>
                Учётная запись (пусто - анонимно)
                <input value={ldapDn} onChange={(e) => setLdapDn(e.target.value)} placeholder="cn=admin,dc=example,dc=com" />
              </label>
              <label style={{ flex: 1 }}>
                Пароль
                <input type="password" value={ldapPass} onChange={(e) => setLdapPass(e.target.value)} />
              </label>
            </div>
            <div className="row">
              <label style={{ flex: 1 }}>
                Откуда искать
                <input value={ldapBase} onChange={(e) => setLdapBase(e.target.value)} placeholder="dc=example,dc=com" />
              </label>
              <label style={{ flex: 1 }}>
                Условие
                <input value={ldapFilter} onChange={(e) => setLdapFilter(e.target.value)} />
              </label>
            </div>
            <p className="hint">
              Запрос идёт <b>с этой машины</b>, выбора «откуда» здесь нет: готовый клиент
              каталога не умеет работать через SSH-канал. Показываются первые 50 записей -
              каталог организации отдаёт их тысячами.
            </p>
            <button
              className="primary"
              disabled={busy}
              onClick={() =>
                void run(() =>
                  window.api.tools.ldap({
                    url: ldapUrl,
                    bindDn: ldapDn,
                    password: ldapPass,
                    base: ldapBase,
                    filter: ldapFilter
                  })
                )
              }
            >
              {busy ? 'Спрашиваю каталог…' : 'Найти'}
            </button>
          </div>
        )}

        {tab === 'diff' && (
          <div className="tools-pane">
            <DiffSide label="Первый файл" value={diffA} onChange={setDiffA} sessions={connectedSessions} />
            <DiffSide label="Второй файл" value={diffB} onChange={setDiffB} sessions={connectedSessions} />
            <p className="hint">
              Каждая сторона - эта машина или любой подключённый сервер. Смысл именно в этом:
              вопрос обычно звучит как «тот же ли конфиг на двух серверах» или «доехала ли
              правка», а не «сравни два файла у себя».
            </p>
            <button
              className="primary"
              disabled={busy || !diffA.path.trim() || !diffB.path.trim()}
              onClick={() => void run(() => window.api.tools.diff(diffA, diffB))}
            >
              {busy ? 'Сравниваю…' : 'Сравнить'}
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
        {out != null &&
          (isDiff(out) ? (
            <Suspense fallback={<div className="hint">Готовлю сравнение…</div>}>
              <DiffView d={out} />
            </Suspense>
          ) : (
            <JsonOut value={out} />
          ))}

        {/* Кнопка закрытия - только у модального варианта. У вкладки для этого есть
            крестик в общей полосе, и вторая такая же рядом только путала бы. */}
        {onClose && (
          <div className="modal-actions">
            <button onClick={onClose}>Закрыть</button>
          </div>
        )}
        </div>
      </div>
    </div>
  )
}
