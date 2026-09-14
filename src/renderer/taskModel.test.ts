import { describe, it, expect } from 'vitest'
import {
  applyProgress,
  moveStep,
  newStep,
  progressFromReport,
  runSummary,
  stepLabel,
  taskProblem,
  type ServerProgress,
  type TaskDef
} from './taskModel'

const task = (over: Partial<TaskDef> = {}): TaskDef => ({
  name: 'Выкладка',
  serverIds: ['a'],
  steps: [{ ...newStep('command'), command: 'uptime' }],
  ...over
})

describe('шаги задачи', () => {
  it('новый шаг получает осмысленные умолчания', () => {
    expect(newStep('service')).toMatchObject({ kind: 'service', action: 'restart', when: 'success', retries: 0 })
    expect(newStep('healthcheck')).toMatchObject({ check: 'http', attempts: 3, intervalSec: 5 })
    expect(newStep('command').id).not.toBe(newStep('command').id)
  })

  it('название шага - своё или из того, что он делает', () => {
    expect(stepLabel({ ...newStep('docker'), container: 'web', action: 'restart' })).toBe('Контейнер web: restart')
    expect(stepLabel({ ...newStep('command'), command: 'apt update\napt upgrade', name: '' })).toBe('Команда: apt update')
    expect(stepLabel({ ...newStep('command'), name: '  Обновить пакеты ' })).toBe('Обновить пакеты')
  })

  it('перестановка не выходит за края', () => {
    const steps = [newStep('command'), newStep('upload'), newStep('service')]
    expect(moveStep(steps, 1, -1).map((s) => s.kind)).toEqual(['upload', 'command', 'service'])
    expect(moveStep(steps, 0, -1)).toBe(steps)
    expect(moveStep(steps, 2, 1)).toBe(steps)
  })
})

describe('проверка до запуска', () => {
  it('называет первую проблему с номером шага', () => {
    expect(taskProblem(task())).toBeNull()
    expect(taskProblem(task({ name: ' ' }))).toBe('У задачи нет названия')
    expect(taskProblem(task({ serverIds: [] }))).toBe('Не выбран ни один сервер')
    expect(taskProblem(task({ steps: [newStep('command'), newStep('upload')] }))).toBe('Шаг 1: пустая команда')
    const port = { ...newStep('healthcheck'), check: 'port' as const, target: '8080' }
    expect(taskProblem(task({ steps: [port] }))).toBe('Шаг 1: порт указывается как адрес:порт')
    const up = { ...newStep('upload'), localPath: 'C:/site', remotePath: '/var/../etc' }
    expect(taskProblem(task({ steps: [up] }))).toContain('«..»')
  })
})

describe('ход прогона', () => {
  it('собирается из событий и не трогает чужой прогон', () => {
    let s: ServerProgress[] = []
    s = applyProgress(s, { runId: 'r', serverId: 'a', name: 'web-1', step: null, state: 'connecting' }, 'r', 2)
    s = applyProgress(s, { runId: 'r', serverId: 'a', name: 'web-1', step: 0, state: 'running' }, 'r', 2)
    s = applyProgress(s, { runId: 'r', serverId: 'a', name: 'web-1', step: 0, state: 'done', output: 'ok' }, 'r', 2)
    s = applyProgress(s, { runId: 'другой', serverId: 'a', name: 'web-1', step: 1, state: 'failed' }, 'r', 2)
    expect(s).toHaveLength(1)
    expect(s[0].state).toBe('running')
    expect(s[0].steps).toEqual([{ state: 'done', output: 'ok' }, { state: 'pending' }])
    s = applyProgress(s, { runId: 'r', serverId: 'a', name: 'web-1', step: null, state: 'done' }, 'r', 2)
    expect(s[0].state).toBe('done')
  })

  it('итог словами', () => {
    expect(runSummary([{ state: 'done' }, { state: 'done' }, { state: 'failed' }, { state: 'skipped' }])).toBe(
      'готово 2 · ошибка 1 · пропущено 1'
    )
    expect(runSummary([])).toBe('нет серверов')
  })
})

describe('отчёт прогона', () => {
  it('раскладывает шаги по номерам, недошедшие остаются в ожидании', () => {
    const p = progressFromReport(
      {
        runId: 'r',
        taskName: 't',
        dryRun: false,
        cancelled: false,
        startedAt: 0,
        finishedAt: 1,
        servers: [
          {
            serverId: 'a',
            name: 'web-1',
            state: 'failed',
            ms: 5,
            steps: [{ index: 1, label: 'x', state: 'failed', output: 'код 1', ms: 1, attempts: 1 }]
          }
        ]
      },
      3
    )
    expect(p[0].steps.map((s) => s.state)).toEqual(['pending', 'failed', 'pending'])
    expect(p[0].steps[1].output).toBe('код 1')
  })
})
