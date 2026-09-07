import { useEffect, useRef, useState } from 'react'
import type { ServerHardware, ServerMetrics, WorkspaceTool } from '../../shared/types'
import { Icon } from './Icon'
import { errText } from '../errText'

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`
  const kb = n / 1024
  if (kb < 1024) return `${kb.toFixed(1)} KiB`
  const mb = kb / 1024
  if (mb < 1024) return `${mb.toFixed(1)} MiB`
  return `${(mb / 1024).toFixed(2)} GiB`
}

function fmtRate(bps: number): string {
  if (bps < 1024) return `${bps.toFixed(0)} B/s`
  const k = bps / 1024
  if (k < 1024) return `${k.toFixed(1)} KiB/s`
  return `${(k / 1024).toFixed(1)} MiB/s`
}

function fmtKb(kb: number): string {
  const mb = kb / 1024
  if (mb < 1024) return `${mb.toFixed(0)} МБ`
  return `${(mb / 1024).toFixed(1)} ГБ`
}

function fmtUptime(sec: number): string {
  const d = Math.floor(sec / 86400)
  const h = Math.floor((sec % 86400) / 3600)
  const m = Math.floor((sec % 3600) / 60)
  if (d > 0) return `${d}д ${h}ч`
  if (h > 0) return `${h}ч ${m}м`
  return `${m}м`
}

function tone(pct: number): 'ok' | 'warn' | 'bad' {
  if (pct < 60) return 'ok'
  if (pct < 85) return 'warn'
  return 'bad'
}

function loadTone(load: number, cores: number): 'ok' | 'warn' | 'bad' {
  const ratio = cores > 0 ? load / cores : load
  if (ratio < 0.7) return 'ok'
  if (ratio < 1.0) return 'warn'
  return 'bad'
}

function loadLabel(load: number, cores: number): string {
  const ratio = cores > 0 ? load / cores : load
  if (ratio < 0.5) return 'легко'
  if (ratio < 0.85) return 'умеренно'
  if (ratio < 1.0) return 'напряжённо'
  return 'перегруз'
}

function Gauge({ pct, label, detail }: { pct: number; label: string; detail: string }): JSX.Element {
  const clamped = Math.min(100, Math.max(0, pct))
  const t = tone(clamped)
  return (
    <div className={'srv-gauge-card tone-' + t}>
      <div
        className="srv-gauge-ring"
        style={{ ['--pct' as string]: String(clamped) }}
        aria-hidden
      >
        <span className="srv-gauge-val">{Math.round(clamped)}%</span>
      </div>
      <div className="srv-gauge-meta">
        <span className="srv-gauge-label">{label}</span>
        <span className="srv-gauge-detail">{detail}</span>
      </div>
    </div>
  )
}

/**
 * Одна строка средней загрузки.
 *
 * Раньше здесь было голое `0.26` и полоска. Число это — среднее количество задач, которые
 * хотели считаться, и само по себе оно не говорит ничего: 0.26 на одноядерной машине это
 * четверть мощности, а на шестнадцати — почти простой. Поэтому теперь сказано прямо:
 * сколько ядер из скольких занято.
 */
function LoadRow({
  label,
  load,
  cores
}: {
  label: string
  load: number
  cores: number
}): JSX.Element {
  const cap = cores > 0 ? (load / cores) * 100 : 0
  const t = loadTone(load, cores)
  return (
    <div className="srv-load-row">
      <span className="srv-load-label">{label}</span>
      <div className="srv-load-track" title={`${load.toFixed(2)} задач в среднем на ${cores} ядрах`}>
        <div className={'srv-load-fill tone-' + t} style={{ width: `${Math.min(100, cap)}%` }} />
      </div>
      <span className="srv-load-num mono">
        {load.toFixed(2)}
        <span className="srv-load-of"> из {cores}</span>
      </span>
      <span className={'srv-load-tag tone-' + t}>{loadLabel(load, cores)}</span>
    </div>
  )
}

function MetricsDashboard({ m }: { m: ServerMetrics }): JSX.Element {
  const memPct = m.memTotalKb > 0 ? (m.memUsedKb / m.memTotalKb) * 100 : 0
  const memFreeKb = m.memTotalKb > m.memUsedKb ? m.memTotalKb - m.memUsedKb : 0
  // Средней загрузки в Windows нет как понятия — не «ноль», а нечего показывать.
  const hasLoad = m.platform !== 'windows'
  const diskLabel = 'Диск ' + (m.diskLabel ?? '/')
  // Второй и дальше том показываем отдельно: у сервера редко один диск, и занятость
  // системного ничего не говорит о том, где на самом деле кончается место.
  const extraVolumes = (m.volumes ?? []).filter((v) => v.mount !== m.diskLabel)
  // Гигабайты вместо голого процента: «занято 12%» не отвечает на вопрос, который на
  // самом деле задают диску, — сколько осталось. У памяти рядом так и показано.
  const mainVolume = (m.volumes ?? []).find((v) => v.mount === m.diskLabel)
  const diskDetail = mainVolume
    ? `${fmtKb(mainVolume.usedKb)} / ${fmtKb(mainVolume.sizeKb)}`
    : `занято ${m.diskPct}%`

  return (
    <div className="srv-dash">
      <div className="srv-dash-gauges">
        <Gauge label="CPU" pct={m.cpuPct} detail={`${m.cpuPct}% · ${m.cores} яд.`} />
        <Gauge
          label="RAM"
          pct={memPct}
          detail={`${fmtKb(m.memUsedKb)} / ${fmtKb(m.memTotalKb)}`}
        />
        <Gauge label={diskLabel} pct={m.diskPct} detail={diskDetail} />
        <div className="srv-dash-summary">
          <div className="srv-dash-summary-row">
            <span className="srv-dash-summary-k">Свободно RAM</span>
            <span className="srv-dash-summary-v mono">{fmtKb(memFreeKb)}</span>
          </div>
          {hasLoad && (
            <div className="srv-dash-summary-row">
              <span className="srv-dash-summary-k">Занято ядер</span>
              <span className={'srv-dash-summary-v mono tone-' + loadTone(m.load[0], m.cores)}>
                {m.load[0].toFixed(1)} из {m.cores}
              </span>
            </div>
          )}
          <div className="srv-dash-summary-row">
            <span className="srv-dash-summary-k">Ядер</span>
            <span className="srv-dash-summary-v mono">{m.cores}</span>
          </div>
        </div>
      </div>
      {hasLoad && (
        <div className="srv-load-block">
          <div className="srv-load-head">
            <span>Средняя загрузка</span>
            <span className="srv-load-hint">
              сколько ядер из {m.cores} занято работой в среднем
            </span>
          </div>
          <div className="srv-load-summary">
            Прямо сейчас занято <b>{m.load[0].toFixed(1)}</b> из {m.cores} —{' '}
            <span className={'tone-' + loadTone(m.load[0], m.cores)}>
              {loadLabel(m.load[0], m.cores)}
            </span>
            . Ниже — как было в среднем за минуту, пять и пятнадцать.
          </div>
          <LoadRow label="1 мин" load={m.load[0]} cores={m.cores} />
          <LoadRow label="5 мин" load={m.load[1]} cores={m.cores} />
          <LoadRow label="15 мин" load={m.load[2]} cores={m.cores} />
        </div>
      )}
      {extraVolumes.length > 0 && (
        <div className="srv-load-block">
          <div className="srv-load-head">
            <span>Остальные тома</span>
            <span className="srv-load-hint">главный показан кольцом выше</span>
          </div>
          {extraVolumes.map((v) => (
            <div className="srv-load-row" key={v.mount}>
              <span className="srv-load-label mono">{v.mount}</span>
              <div className="srv-load-track" title={`${fmtKb(v.usedKb)} из ${fmtKb(v.sizeKb)}`}>
                <div
                  className={'srv-load-fill tone-' + (v.usePct >= 90 ? 'bad' : v.usePct >= 75 ? 'warn' : 'ok')}
                  style={{ width: `${Math.min(100, v.usePct)}%` }}
                />
              </div>
              <span className="srv-load-num mono">{v.usePct}%</span>
              <span className="srv-load-tag">{fmtKb(v.sizeKb)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

/**
 * Карточки железа: процессор, видео, память.
 *
 * Стоят первыми среди карточек не случайно. Когда открываешь незнакомый сервер, первый
 * вопрос — «что это за машина», и только потом «чем она сейчас занята».
 */
function HardwareCards({
  hw,
  memTotalKb
}: {
  hw: ServerHardware
  memTotalKb: number
}): JSX.Element {
  const cpuLine = [
    hw.cores ? `${hw.cores} ядер` : null,
    hw.threads ? `${hw.threads} потоков` : null,
    hw.mhz ? `${(hw.mhz / 1000).toFixed(1)} ГГц` : null
  ]
    .filter(Boolean)
    .join(' · ')

  const memLine = [hw.memType, hw.memSpeed].filter(Boolean).join(' · ')

  return (
    <>
      {hw.cpu && (
        <div className="srv-overview-card">
          <span className="srv-overview-k">Процессор</span>
          <span className="srv-overview-v">{hw.cpu}</span>
          {cpuLine && <span className="srv-overview-sub">{cpuLine}</span>}
        </div>
      )}
      {hw.gpus.length > 0 && (
        <div className="srv-overview-card">
          <span className="srv-overview-k">Видео</span>
          {hw.gpus.map((g) => (
            <span key={g.name} className="srv-overview-v" title={g.name}>
              {g.name}
              <span className="srv-overview-sub">
                {/* Незагруженный драйвер — законное состояние, а не ошибка: видео
                    работает в базовом режиме, без ускорения. Так и пишем. */}
                {g.driver ? `драйвер ${g.driver}` : 'драйвер не загружен'}
              </span>
            </span>
          ))}
        </div>
      )}
      <div className="srv-overview-card">
        <span className="srv-overview-k">Память</span>
        <span className="srv-overview-v mono">{fmtKb(memTotalKb)}</span>
        {memLine ? (
          <span className="srv-overview-sub">{memLine}</span>
        ) : (
          hw.memWhy && <span className="srv-overview-sub">скорость: {hw.memWhy}</span>
        )}
      </div>
      {hw.virt && (
        <div className="srv-overview-card">
          <span className="srv-overview-k">Виртуализация</span>
          <span className="srv-overview-v">{hw.virt}</span>
        </div>
      )}
    </>
  )
}

function OverviewCards({
  m,
  hw,
  netRxRate,
  netTxRate,
  onGoTool
}: {
  m: ServerMetrics
  hw: ServerHardware | null
  netRxRate: number | null
  netTxRate: number | null
  onGoTool?: (tool: WorkspaceTool) => void
}): JSX.Element {
  const failed = m.failedServices
  const svcTone =
    failed === undefined ? 'muted' : failed === 0 ? 'ok' : 'bad'
  const svcLabel =
    failed === undefined
      ? 'Список упавших служб недоступен'
      : failed === 0
        ? 'Все сервисы в порядке'
        : `${failed} упало`

  let dockerLabel = 'Docker не установлен'
  let dockerTone: 'muted' | 'ok' | 'warn' = 'muted'
  if (m.dockerAvailable) {
    const run = m.dockerRunning ?? 0
    const stop = m.dockerStopped ?? 0
    dockerLabel = `${run} работает · ${stop} остановлено`
    dockerTone = stop > 0 ? 'warn' : 'ok'
  }

  const netDetail =
    m.netIface && (netRxRate !== null || netTxRate !== null)
      ? `${m.netIface}: ↓ ${netRxRate !== null ? fmtRate(netRxRate) : '—'} · ↑ ${netTxRate !== null ? fmtRate(netTxRate) : '—'}`
      : m.netRxBytes !== undefined
        ? `${m.netIface ?? 'iface'}: ↓ ${fmtBytes(m.netRxBytes)} · ↑ ${fmtBytes(m.netTxBytes ?? 0)}`
        : 'Интерфейс не определён'

  return (
    <div className="srv-overview-cards">
      {hw && <HardwareCards hw={hw} memTotalKb={m.memTotalKb} />}
      {(m.os || m.kernel) && (
        <div className="srv-overview-card">
          <span className="srv-overview-k">ОС и ядро</span>
          <span className="srv-overview-v">{m.os ?? 'Linux'}</span>
          {m.kernel && <span className="srv-overview-sub mono">{m.kernel}</span>}
        </div>
      )}
      {m.procCount !== undefined && (
        <div className="srv-overview-card">
          <span className="srv-overview-k">Процессы</span>
          <span className="srv-overview-v mono">{m.procCount}</span>
          {onGoTool && (
            <button type="button" className="srv-overview-link" onClick={() => onGoTool('processes')}>
              Открыть список
            </button>
          )}
        </div>
      )}
      <div className="srv-overview-card">
        <span className="srv-overview-k">Сеть</span>
        <span className="srv-overview-v srv-overview-net">{netDetail}</span>
      </div>
      <div className={'srv-overview-card tone-' + svcTone}>
        <span className="srv-overview-k">Systemd</span>
        <span className="srv-overview-v">{svcLabel}</span>
        {onGoTool && failed !== undefined && (
          <button type="button" className="srv-overview-link" onClick={() => onGoTool('services')}>
            Сервисы
          </button>
        )}
      </div>
      <div className={'srv-overview-card tone-' + dockerTone}>
        <span className="srv-overview-k">Docker</span>
        <span className="srv-overview-v">{dockerLabel}</span>
        {onGoTool && m.dockerAvailable && (
          <button type="button" className="srv-overview-link" onClick={() => onGoTool('docker')}>
            Контейнеры
          </button>
        )}
      </div>
    </div>
  )
}

function Bar({ label, pct, sub }: { label: string; pct: number; sub: string }): JSX.Element {
  const t = tone(pct)
  return (
    <div className="mon-metric">
      <div className="mon-metric-head">
        <span>{label}</span>
        <span className="mon-sub">{sub}</span>
      </div>
      <div className="bar">
        <div className={'bar-fill tone-' + t} style={{ width: `${Math.min(100, pct)}%` }} />
      </div>
    </div>
  )
}

function MetricsCompact({ m }: { m: ServerMetrics }): JSX.Element {
  const memPct = m.memTotalKb > 0 ? (m.memUsedKb / m.memTotalKb) * 100 : 0
  const mainVolume = (m.volumes ?? []).find((v) => v.mount === m.diskLabel)
  return (
    <div className="mon-body docked">
      <Bar label="CPU" pct={m.cpuPct} sub={`${m.cpuPct}% · ${m.cores} ядр.`} />
      <Bar label="RAM" pct={memPct} sub={`${fmtKb(m.memUsedKb)} / ${fmtKb(m.memTotalKb)}`} />
      <Bar
        label={'Диск ' + (m.diskLabel ?? '/')}
        pct={m.diskPct}
        sub={
          mainVolume
            ? `${fmtKb(mainVolume.usedKb)} / ${fmtKb(mainVolume.sizeKb)}`
            : `${m.diskPct}%`
        }
      />
      {m.platform !== 'windows' && (
        <div className="mon-load" title="Среднее число задач, ждавших процессор, за 1, 5 и 15 минут">
          Занято ядер: <b>{m.load[0].toFixed(1)}</b> из {m.cores}
          <span className="mon-load-rest">
            {' '}· за 5 мин {m.load[1].toFixed(1)} · за 15 мин {m.load[2].toFixed(1)}
          </span>
        </div>
      )}
    </div>
  )
}

export function MonitorMetrics({
  sessionId,
  compact,
  variant = compact ? 'compact' : 'default',
  showOverviewCards,
  onGoTool
}: {
  sessionId: string
  /** @deprecated — используй variant="compact" */
  compact?: boolean
  variant?: 'default' | 'compact' | 'dashboard'
  showOverviewCards?: boolean
  onGoTool?: (tool: WorkspaceTool) => void
}): JSX.Element {
  const [m, setM] = useState<ServerMetrics | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [fresh, setFresh] = useState(false)
  const [netRxRate, setNetRxRate] = useState<number | null>(null)
  const [netTxRate, setNetTxRate] = useState<number | null>(null)
  // Железо спрашиваем один раз за сессию: модель процессора не меняется, а метрики
  // обновляются каждые несколько секунд — тянуть это по таймеру значило бы впустую
  // гонять канал. Ошибку не показываем: сведения о железе приятны, но не обязательны,
  // и падать из-за них панели незачем.
  const [hw, setHw] = useState<ServerHardware | null>(null)
  const aliveRef = useRef(true)
  const prevNetRef = useRef<{ rx?: number; tx?: number; at: number } | null>(null)

  useEffect(() => {
    let ушли = false
    void window.api.session
      .sysinfo(sessionId)
      .then((v) => {
        if (!ушли) setHw(v)
      })
      .catch(() => {
        /* железо не узнали — панель работает и без него */
      })
    return () => {
      ушли = true
    }
  }, [sessionId])

  useEffect(() => {
    aliveRef.current = true
    prevNetRef.current = null
    setNetRxRate(null)
    setNetTxRate(null)
    const tick = async (): Promise<void> => {
      try {
        const data = await window.api.session.monitor(sessionId)
        if (!aliveRef.current) return
        if (data.ok) {
          const now = Date.now()
          if (data.netRxBytes !== undefined && data.netTxBytes !== undefined) {
            const prev = prevNetRef.current
            if (prev?.rx !== undefined && prev.tx !== undefined) {
              const dt = (now - prev.at) / 1000
              if (dt > 0.2) {
                setNetRxRate(Math.max(0, (data.netRxBytes - prev.rx) / dt))
                setNetTxRate(Math.max(0, (data.netTxBytes - prev.tx) / dt))
              }
            }
            prevNetRef.current = { rx: data.netRxBytes, tx: data.netTxBytes, at: now }
          }
          setM(data)
          setErr(null)
          setFresh(true)
          window.setTimeout(() => {
            if (aliveRef.current) setFresh(false)
          }, 600)
        } else setErr(data.error || 'Не удалось получить метрики')
      } catch (e) {
        if (aliveRef.current) setErr(errText(e))
      }
    }
    void tick()
    const id = window.setInterval(tick, 3000)
    return () => {
      aliveRef.current = false
      window.clearInterval(id)
    }
  }, [sessionId])

  const isDashboard = variant === 'dashboard'
  const isCompact = variant === 'compact'

  return (
    <div
      className={
        'ws-metrics' +
        (isCompact ? ' compact' : '') +
        (isDashboard ? ' dashboard' : '')
      }
    >
      <div className="ws-metrics-head">
        <span className="ws-metrics-title">
          <Icon name="monitor" size={14} />
          {isDashboard ? 'Загрузка сервера' : 'Ресурсы'}
        </span>
        <div className="ws-metrics-meta">
          {m?.os && isDashboard && (
            <span className="mon-os" title={m.kernel}>
              {m.os}
            </span>
          )}
          {m && (
            <>
              <span className={'srv-live' + (fresh ? ' ping' : '')} title="Обновление каждые 3 с">
                live
              </span>
              <span className="mon-uptime" title="Uptime">
                <Icon name="arrow-up" size={11} /> {fmtUptime(m.uptimeSec)}
              </span>
            </>
          )}
        </div>
      </div>
      {err && <div className="hint ws-metrics-err">{err}</div>}
      {!err && !m && <div className="hint ws-metrics-err">Сбор метрик…</div>}
      {m && (isDashboard ? <MetricsDashboard m={m} /> : <MetricsCompact m={m} />)}
      {m && showOverviewCards && (
        <OverviewCards m={m} hw={hw} netRxRate={netRxRate} netTxRate={netTxRate} onGoTool={onGoTool} />
      )}
    </div>
  )
}
