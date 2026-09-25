import { describe, expect, it, vi } from 'vitest'

const { ask } = vi.hoisted(() => ({ ask: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask }))

import { confirmAction } from './confirmDialog'

describe('confirmAction', () => {
  it('ждёт ответа человека и возвращает именно его', async () => {
    ask.mockResolvedValue(false)
    expect(await confirmAction('Удалить сервер «web-02»?')).toBe(false)
    ask.mockResolvedValue(true)
    expect(await confirmAction('Удалить сервер «web-02»?')).toBe(true)
  })

  it('спрашивает словами, а не кнопками Yes/No', async () => {
    ask.mockResolvedValue(true)
    await confirmAction('Удалить?')
    expect(ask).toHaveBeenLastCalledWith('Удалить?', expect.objectContaining({ okLabel: 'Да', cancelLabel: 'Отмена' }))
  })

  it('окно не показалось - значит «нет», а не молчаливое «да»', async () => {
    ask.mockImplementation(async () => {
      throw new Error('dialog.confirm not allowed. Command not found')
    })
    expect(await confirmAction('Удалить?')).toBe(false)
  })
})
