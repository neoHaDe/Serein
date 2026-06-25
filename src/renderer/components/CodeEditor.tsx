import { useEffect, useRef, useState } from 'react'
import { EditorView, keymap } from '@codemirror/view'
import { basicSetup } from 'codemirror'
import { oneDark } from '@codemirror/theme-one-dark'
import { languageFor } from '../editorLang'
import { useSettings } from '../SettingsContext'

interface Props {
  sessionId: string
  remotePath: string
  fileName: string
  active: boolean
  onDirtyChange: (dirty: boolean) => void
}

type Phase = 'loading' | 'ready' | 'binary' | 'toolarge' | 'error'
type SaveState = 'idle' | 'saving' | 'saved' | 'error'

export function CodeEditor({ sessionId, remotePath, fileName, active, onDirtyChange }: Props): JSX.Element {
  const hostRef = useRef<HTMLDivElement>(null)
  const viewRef = useRef<EditorView | null>(null)
  const metaRef = useRef({ mode: 0o644, mtime: 0, eol: 'lf' as 'lf' | 'crlf', saved: '' })
  const saveRef = useRef<() => void>(() => {})
  const [phase, setPhase] = useState<Phase>('loading')
  const [saveState, setSaveState] = useState<SaveState>('idle')
  const [msg, setMsg] = useState<string>()
  const { settings } = useSettings()

  // Загрузка файла и создание редактора.
  useEffect(() => {
    let cancelled = false
    setPhase('loading')
    window.api.sftp
      .readFile(sessionId, remotePath)
      .then((res) => {
        if (cancelled) return
        if (res.tooLarge) return setPhase('toolarge')
        if (res.binary) return setPhase('binary')
        metaRef.current = { mode: res.mode, mtime: res.mtime, eol: res.eol, saved: res.content }

        const view = new EditorView({
          doc: res.content,
          parent: hostRef.current!,
          extensions: [
            // Наш Ctrl/Cmd+S имеет приоритет над дефолтными биндингами.
            keymap.of([{ key: 'Mod-s', preventDefault: true, run: () => (saveRef.current(), true) }]),
            basicSetup,
            languageFor(fileName) ?? [],
            oneDark,
            EditorView.theme({
              '&': { height: '100%', fontSize: `${settings.fontSize}px` },
              '.cm-scroller': { fontFamily: settings.fontFamily, overflow: 'auto' }
            }),
            EditorView.updateListener.of((u) => {
              if (u.docChanged) {
                const dirty = u.state.doc.toString() !== metaRef.current.saved
                onDirtyChange(dirty)
                setSaveState((s) => (s === 'saved' ? 'idle' : s))
              }
            })
          ]
        })
        viewRef.current = view
        setPhase('ready')
        // Хост был display:none во время загрузки — пересчитать размеры после показа.
        requestAnimationFrame(() => {
          view.requestMeasure()
          if (active) view.focus()
        })
      })
      .catch((e) => {
        if (cancelled) return
        setPhase('error')
        setMsg((e as Error).message)
      })

    return () => {
      cancelled = true
      viewRef.current?.destroy()
      viewRef.current = null
    }
    // Пересоздаём только при смене файла/сессии.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId, remotePath])

  // При появлении вкладки — пересчитать размеры и вернуть фокус.
  useEffect(() => {
    if (active && viewRef.current) {
      viewRef.current.requestMeasure()
      viewRef.current.focus()
    }
  }, [active])

  // Сохранение (через ref, чтобы keymap всегда звал актуальную версию).
  saveRef.current = async (): Promise<void> => {
    const view = viewRef.current
    if (!view) return
    const text = view.state.doc.toString()
    const m = metaRef.current
    setSaveState('saving')
    setMsg(undefined)
    try {
      let res = await window.api.sftp.writeFile(sessionId, remotePath, text, m.mode, m.mtime, m.eol)
      if (res.conflict) {
        if (!confirm('Файл изменился на сервере с момента открытия. Перезаписать своей версией?')) {
          setSaveState('idle')
          return
        }
        res = await window.api.sftp.writeFile(sessionId, remotePath, text, m.mode, 0, m.eol)
      }
      if (res.ok) {
        m.saved = text
        if (res.mtime) m.mtime = res.mtime
        onDirtyChange(false)
        setSaveState('saved')
      } else {
        setSaveState('error')
        setMsg(res.error || 'Не удалось сохранить')
      }
    } catch (e) {
      setSaveState('error')
      setMsg((e as Error).message)
    }
  }

  return (
    <div className="code-editor">
      <div className="code-editor-bar">
        <span className="ce-path" title={remotePath}>{remotePath}</span>
        <span className="ce-spacer" />
        <span className={'ce-state ' + saveState}>
          {saveState === 'saving'
            ? '⟳ Сохранение…'
            : saveState === 'saved'
              ? '✓ Сохранено'
              : saveState === 'error'
                ? `⚠ ${msg ?? 'Ошибка'}`
                : ''}
        </span>
        <button className="secondary ce-save" onClick={() => saveRef.current()} disabled={phase !== 'ready'}>
          💾 Сохранить (Ctrl+S)
        </button>
      </div>

      {phase === 'loading' && <div className="ce-msg">Загрузка файла…</div>}
      {phase === 'binary' && <div className="ce-msg">Это бинарный файл — откройте его скачиванием.</div>}
      {phase === 'toolarge' && <div className="ce-msg">Файл слишком большой для встроенного редактора (&gt; 5 МБ).</div>}
      {phase === 'error' && <div className="ce-msg error">Ошибка: {msg}</div>}
      <div className="ce-host" ref={hostRef} style={{ display: phase === 'ready' ? 'block' : 'none' }} />
    </div>
  )
}
