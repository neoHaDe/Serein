import { useCallback, useEffect, useRef, useState } from 'react'
import type { TransferItem } from '../../shared/types'
import { fmtEta, fmtSize, fmtSpeed, nextRate, parentOfAny, type RateSample } from '../sftpPaths'

const LIVE: TransferItem['state'][] = ['queued', 'active', 'paused']

export function isLive(t: TransferItem): boolean {
  return LIVE.includes(t.state)
}

/**
 * Очередь передач одной сессии: события бэкенда, скорость, пауза, отмена, повтор.
 *
 * Только своей сессии. Раньше панель слушала передачи всех сессий сразу: с двумя открытыми
 * серверами каждая показывала чужие, а «Повторить» отправляло чужую передачу через свою
 * сессию - то есть на другой сервер.
 *
 * `onFinished` зовётся, когда передача кончилась и списку файлов пора обновиться.
 */
export function useSftpTransfers(
  sessionId: string,
  onFinished: (item: TransferItem) => void
): {
  transfers: TransferItem[]
  cancel: (id: string) => void
  pause: (id: string) => void
  resume: (id: string) => void
  retry: (t: TransferItem) => void
  clearFinished: () => void
} {
  const [transfers, setTransfers] = useState<TransferItem[]>([])
  const rates = useRef(new Map<string, RateSample>())
  const finished = useRef(onFinished)
  finished.current = onFinished

  useEffect(() => {
    return window.api.sftp.onTransfer((item) => {
      if (item.sessionId !== sessionId) return
      const { bps, sample } = nextRate(rates.current.get(item.id), item.state, item.transferred, Date.now())
      if (sample === null) rates.current.delete(item.id)
      else if (sample) rates.current.set(item.id, sample)
      const nextItem = { ...item, speedBps: bps }
      setTransfers((prev) => {
        const idx = prev.findIndex((t) => t.id === nextItem.id)
        if (idx === -1) return [...prev, nextItem]
        const next = [...prev]
        next[idx] = nextItem
        return next
      })
      if (item.state === 'done' || item.state === 'error' || item.state === 'canceled') finished.current(item)
    })
  }, [sessionId])

  const cancel = useCallback((id: string): void => {
    setTransfers((prev) => prev.map((t) => (t.id === id && isLive(t) ? { ...t, state: 'canceled' as const } : t)))
    void window.api.sftp.cancelTransfer(id)
  }, [])
  const pause = useCallback((id: string): void => {
    setTransfers((prev) =>
      prev.map((t) => (t.id === id && t.state === 'active' ? { ...t, state: 'paused' as const, speedBps: 0 } : t))
    )
    void window.api.sftp.pauseTransfer(id)
  }, [])
  const resume = useCallback((id: string): void => {
    setTransfers((prev) => prev.map((t) => (t.id === id && t.state === 'paused' ? { ...t, state: 'active' as const } : t)))
    void window.api.sftp.resumeTransfer(id)
  }, [])
  const retry = useCallback((t: TransferItem): void => {
    setTransfers((prev) => prev.filter((x) => x.id !== t.id))
    if (t.direction === 'download') {
      void window.api.sftp.downloadTo(t.sessionId, t.remotePath, parentOfAny(t.localPath))
    } else {
      void window.api.sftp.uploadPaths(t.sessionId, parentOfAny(t.remotePath), [t.localPath])
    }
  }, [])
  const clearFinished = useCallback((): void => {
    setTransfers((prev) => prev.filter(isLive))
  }, [])

  return { transfers, cancel, pause, resume, retry, clearFinished }
}

function stateText(t: TransferItem): string {
  switch (t.state) {
    case 'done':
      return '✓'
    case 'error':
      return '⚠'
    case 'canceled':
      return '⊘'
    case 'queued':
      return 'ожидание'
    case 'paused':
      return 'пауза'
    default:
      return [
        t.size ? `${fmtSize(t.transferred)} / ${fmtSize(t.size)}` : fmtSize(t.transferred),
        fmtSpeed(t.speedBps),
        fmtEta(t.size, t.transferred, t.speedBps),
      ]
        .filter(Boolean)
        .join(' · ')
  }
}

/** Список передач под панелью файлов. */
export function SftpTransferQueue({
  transfers,
  onCancel,
  onPause,
  onResume,
  onRetry,
  onClear,
}: {
  transfers: TransferItem[]
  onCancel: (id: string) => void
  onPause: (id: string) => void
  onResume: (id: string) => void
  onRetry: (t: TransferItem) => void
  onClear: () => void
}): JSX.Element | null {
  if (transfers.length === 0) return null
  const active = transfers.filter(isLive).length
  return (
    <div className="sftp-queue">
      <div className="sftp-queue-head">
        <span>Передачи ({active} активны)</span>
        <button className="mini" title="Очистить завершённые" onClick={onClear}>
          Очистить
        </button>
      </div>
      <div className="sftp-queue-list">
        {transfers.map((t) => (
          <div key={t.id} className={'sftp-queue-item ' + t.state}>
            <span className="q-dir">{t.direction === 'upload' ? '⬆' : '⬇'}</span>
            <div className="q-info">
              <div className="q-name" title={t.error || t.filename}>
                {t.filename}
                {t.state === 'error' && <span className="q-err"> - {t.error}</span>}
              </div>
              {isLive(t) && (
                <div className="bar">
                  <div
                    className="bar-fill"
                    style={{ width: t.size ? `${Math.min(100, (t.transferred / t.size) * 100)}%` : '0%' }}
                  />
                </div>
              )}
            </div>
            <span className="q-state">{stateText(t)}</span>
            <span className="q-actions">
              {t.state === 'active' && (
                <button className="mini" title="Пауза" onClick={() => onPause(t.id)}>
                  ❚❚
                </button>
              )}
              {t.state === 'paused' && (
                <button className="mini" title="Продолжить" onClick={() => onResume(t.id)}>
                  ▶
                </button>
              )}
              {(t.state === 'error' || t.state === 'canceled') && (
                <button className="mini" title="Повторить" onClick={() => onRetry(t)}>
                  ↻
                </button>
              )}
              {isLive(t) && (
                <button className="mini danger" title="Отменить" onClick={() => onCancel(t.id)}>
                  ✕
                </button>
              )}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}
