import { useRef, useState } from 'react'
import type { KIPrompt } from '../../shared/types'

interface Props {
  sessionId: string
  prompts: KIPrompt[]
  onSubmit: (answers: string[]) => void
  onCancel: () => void
}

export function KiModal({ prompts, onSubmit, onCancel }: Props): JSX.Element {
  const [answers, setAnswers] = useState<string[]>(prompts.map(() => ''))
  const firstRef = useRef<HTMLInputElement>(null)

  const update = (i: number, val: string): void => {
    setAnswers((prev) => prev.map((a, idx) => (idx === i ? val : a)))
  }

  const submit = (): void => onSubmit(answers)

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Требуется дополнительная аутентификация</h2>
        <p style={{ color: 'var(--muted)', fontSize: 12, marginTop: 0, marginBottom: 16 }}>
          Сервер запрашивает дополнительные данные (2FA / OTP / пароль).
        </p>
        {prompts.map((p, i) => (
          <label key={i}>
            {p.prompt}
            <input
              ref={i === 0 ? firstRef : undefined}
              autoFocus={i === 0}
              type={p.echo ? 'text' : 'password'}
              value={answers[i]}
              onChange={(e) => update(i, e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  if (i < prompts.length - 1) {
                    // фокус на следующий
                  } else {
                    submit()
                  }
                }
                if (e.key === 'Escape') onCancel()
              }}
            />
          </label>
        ))}
        <div className="modal-actions">
          <button className="secondary" onClick={onCancel}>Отмена</button>
          <button className="primary" onClick={submit}>Подтвердить</button>
        </div>
      </div>
    </div>
  )
}
