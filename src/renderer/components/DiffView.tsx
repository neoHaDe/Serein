import { useMemo } from 'react'
import { highlightTree } from '@lezer/highlight'
import { HighlightStyle } from '@codemirror/language'
import { tags as t } from '@lezer/highlight'
import { languageFor } from '../editorLang'
import type { LanguageSupport, StreamLanguage } from '@codemirror/language'

/**
 * Сравнение двух файлов в человеческом виде.
 *
 * До этого панель показывала сырой JSON: массив объектов с тегами и номерами строк.
 * Формально там всё было, читать это было нельзя. Здесь то же самое, но так, как
 * сравнение показывают везде: два столбца номеров, знак в жёлобе, цвет строки.
 *
 * Компонент грузится по требованию: подсветка тянет за собой разбор языков, а платить
 * за него при запуске должны только те, кто сравнением пользуется.
 */

interface Side {
  label: string
  sha256: string
  bytes: number
  lines: number
}

export interface DiffResult {
  a: string
  b: string
  sideA?: Side
  sideB?: Side
  same: boolean
  added: number
  removed: number
  lines: { tag: 'add' | 'del' | 'eq'; a: number | null; b: number | null; text: string }[]
  note?: string
  truncated?: string
}

/**
 * Цвета для подсветки.
 *
 * Свой набор, а не тема редактора: у той фон свой, а строки диффа уже покрашены в
 * зелёный и красный, и чужой фон поверх них дал бы кашу. Берём только цвет текста.
 */
const highlight = HighlightStyle.define([
  { tag: t.keyword, color: 'var(--syn-keyword)' },
  { tag: [t.string, t.special(t.string)], color: 'var(--syn-string)' },
  { tag: [t.number, t.bool, t.null], color: 'var(--syn-number)' },
  { tag: t.comment, color: 'var(--syn-comment)', fontStyle: 'italic' },
  { tag: [t.propertyName, t.attributeName], color: 'var(--syn-prop)' },
  { tag: [t.typeName, t.className, t.namespace], color: 'var(--syn-type)' },
  { tag: [t.function(t.variableName), t.definition(t.variableName)], color: 'var(--syn-func)' },
  { tag: [t.operator, t.punctuation], color: 'var(--syn-punct)' },
  { tag: t.invalid, color: 'var(--danger)' }
])

type Lang = LanguageSupport | StreamLanguage<unknown> | undefined

/** Разбирает строку на куски с классами подсветки. Не вышло - отдаём как есть. */
function paint(text: string, lang: Lang): JSX.Element[] | string {
  if (!lang || !text) return text
  try {
    // У обоих видов языка разбор лежит в одном месте, но добираются до него по-разному.
    const language = 'language' in lang ? lang.language : lang
    const tree = language.parser.parse(text)
    const out: JSX.Element[] = []
    let at = 0
    highlightTree(tree, highlight, (from, to, cls) => {
      if (from > at) out.push(<span key={at}>{text.slice(at, from)}</span>)
      out.push(
        <span key={from} className={cls}>
          {text.slice(from, to)}
        </span>
      )
      at = to
    })
    if (at < text.length) out.push(<span key={at}>{text.slice(at)}</span>)
    return out.length ? out : text
  } catch {
    // Подсветка - украшение. Если разбор споткнулся, сравнение всё равно должно
    // показаться: без цвета оно читается, без строк - нет.
    return text
  }
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} Б`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} КБ`
  return `${(n / 1024 / 1024).toFixed(1)} МБ`
}

/** Короткий вид суммы: целиком её никто не читает, а первых знаков хватает для сверки. */
function shortHash(h: string): string {
  return h.slice(0, 12)
}

export function DiffView({ d }: { d: DiffResult }): JSX.Element {
  // Язык берём по имени первого файла: сравнивают почти всегда одноимённые.
  const lang = useMemo(() => languageFor(d.sideA?.label ?? d.a) as Lang, [d.sideA?.label, d.a])
  const sameHash = d.sideA && d.sideB && d.sideA.sha256 === d.sideB.sha256

  return (
    <div className="diff">
      <div className="diff-heads">
        {[d.sideA, d.sideB].map((s, i) =>
          s ? (
            <div key={i} className={'diff-head ' + (i === 0 ? 'a' : 'b')}>
              <span className="diff-head-name" title={s.label}>
                {s.label}
              </span>
              <span className="diff-head-meta">
                {s.lines} стр · {fmtBytes(s.bytes)}
              </span>
              <span className={'diff-hash' + (sameHash ? ' same' : '')} title={s.sha256}>
                sha256 {shortHash(s.sha256)}
              </span>
            </div>
          ) : null
        )}
      </div>

      {/* Вердикт словами. Он же отвечает на вопрос, ради которого сравнение и открыли. */}
      {d.same ? (
        <div className="diff-verdict same">Файлы совпадают побайтово</div>
      ) : (
        <div className="diff-verdict">
          <span className="diff-add">+{d.added}</span> <span className="diff-del">−{d.removed}</span>
          {sameHash && ' · суммы совпали'}
        </div>
      )}

      {d.note && <div className="diff-note">{d.note}</div>}
      {d.truncated && <div className="diff-note">{d.truncated}</div>}

      {d.lines.length > 0 && (
        <div className="diff-body">
          {d.lines.map((l, i) => (
            <div key={i} className={'diff-row ' + l.tag}>
              <span className="diff-num">{l.a ?? ''}</span>
              <span className="diff-num">{l.b ?? ''}</span>
              <span className="diff-sign">{l.tag === 'add' ? '+' : l.tag === 'del' ? '−' : ' '}</span>
              <span className="diff-text">{paint(l.text, lang)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
