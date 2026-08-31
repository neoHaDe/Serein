import { useEffect, useState } from 'react'
import type { HostKeyRequest, KIPrompt } from '../../shared/types'

/**
 * Два вопроса, которые бэкенд задаёт пользователю посреди подключения: ключ хоста и
 * keyboard-interactive (2FA/OTP).
 *
 * Вынесено из `App.tsx` первым не потому, что самое крупное, а потому что самое
 * самодостаточное: сюда не ведёт ни одна нитка из вкладок, сессий или раскладки. Ровно то,
 * с чего стоит начинать разбор большого компонента — иначе первая же попытка утащит за
 * собой половину файла.
 */

export interface ServerPrompts {
  /**
   * Очередь вопросов о ключе хоста, показываем по одному.
   *
   * Очередь, а не одно значение: подключений может идти несколько сразу (восстановление
   * вкладок при запуске, массовый прогон), и вопрос от второго сервера не должен затирать
   * неотвеченный вопрос от первого — иначе первое подключение зависнет навсегда.
   */
  hostKeyQueue: HostKeyRequest[]
  answerHostKey: (accept: boolean) => void
  ki: { id: string; prompts: KIPrompt[] } | null
  answerKi: (answers: string[]) => void
  cancelKi: () => void
}

export function useServerPrompts(): ServerPrompts {
  const [hostKeyQueue, setHostKeyQueue] = useState<HostKeyRequest[]>([])
  const [ki, setKi] = useState<{ id: string; prompts: KIPrompt[] } | null>(null)

  useEffect(() => window.api.session.onHostKey((p) => setHostKeyQueue((q) => [...q, p])), [])
  useEffect(() => window.api.session.onKi((p) => setKi(p)), [])

  const answerHostKey = (accept: boolean): void => {
    setHostKeyQueue((q) => {
      const req = q[0]
      if (!req) return q
      void window.api.session.respondHostKey(req.requestId, accept)
      return q.slice(1)
    })
  }

  const answerKi = (answers: string[]): void => {
    setKi((cur) => {
      if (cur) void window.api.session.respondKi(cur.id, answers)
      return null
    })
  }

  // Отказ — это тоже ответ. Молча закрыть окно нельзя: подключение на той стороне
  // осталось бы ждать ввода до таймаута.
  const cancelKi = (): void => answerKi([])

  return { hostKeyQueue, answerHostKey, ki, answerKi, cancelKi }
}
