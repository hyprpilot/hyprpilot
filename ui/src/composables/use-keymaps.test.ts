import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { type KeymapsConfig, Modifier } from '@ipc'

const { invokeMock, warnMock } = vi.hoisted(() => ({ invokeMock: vi.fn(), warnMock: vi.fn() }))

vi.mock('@ipc', async () => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (...args: unknown[]) => invokeMock(...args)
}))

vi.mock('@lib', async () => ({
  ...(await vi.importActual<object>('@lib')),
  log: { warn: warnMock, error: vi.fn(), info: vi.fn(), debug: vi.fn() }
}))

import { isEditableTarget } from './use-keymaps'

function defaultKeymaps(): KeymapsConfig {
  return {
    chat: {
      submit: { modifiers: [], key: 'enter' },
      newline: { modifiers: [Modifier.Shift], key: 'enter' }
    },
    approvals: {
      allow: { modifiers: [], key: 'a' },
      deny: { modifiers: [], key: 'd' }
    },
    composer: {
      paste_image: { modifiers: [Modifier.Ctrl], key: 'p' },
      tab_completion: { modifiers: [], key: 'tab' },
      shift_tab: { modifiers: [Modifier.Shift], key: 'tab' },
      history_up: { modifiers: [Modifier.Ctrl], key: 'arrowup' },
      history_down: { modifiers: [Modifier.Ctrl], key: 'arrowdown' }
    },
    palette: {
      open: { modifiers: [Modifier.Ctrl], key: 'k' },
      close: { modifiers: [], key: 'escape' },
      models: { focus: { modifiers: [Modifier.Ctrl], key: 'm' } },
      sessions: { focus: { modifiers: [Modifier.Ctrl], key: 's' } }
    },
    transcript: {}
  }
}

describe('useKeymaps', () => {
  beforeEach(async () => {
    invokeMock.mockReset()
    warnMock.mockReset()
    const mod = await import('./use-keymaps')
    mod.__resetKeymapsForTests()
  })

  afterEach(async () => {
    const mod = await import('./use-keymaps')
    mod.__resetKeymapsForTests()
  })

  it('keymaps is undefined before loadKeymaps resolves', async () => {
    const { useKeymaps } = await import('./use-keymaps')
    const { keymaps } = useKeymaps()
    expect(keymaps.value).toBeUndefined()
  })

  it('loadKeymaps populates the cache from the get_keymaps command', async () => {
    const fixture = defaultKeymaps()
    invokeMock.mockResolvedValueOnce(fixture)

    const { loadKeymaps, useKeymaps } = await import('./use-keymaps')
    await loadKeymaps()

    const { keymaps } = useKeymaps()
    expect(keymaps.value).toEqual(fixture)
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('get_keymaps')
  })

  it('loadKeymaps soft-fails when invoke rejects (no Tauri host)', async () => {
    invokeMock.mockRejectedValueOnce(new Error('tauri host missing'))

    const { loadKeymaps, useKeymaps } = await import('./use-keymaps')
    await loadKeymaps()

    const { keymaps } = useKeymaps()
    expect(keymaps.value).toBeUndefined()
    expect(warnMock).toHaveBeenCalledTimes(1)
  })

  it('multiple useKeymaps calls share the same ref', async () => {
    const fixture = defaultKeymaps()
    invokeMock.mockResolvedValueOnce(fixture)

    const { loadKeymaps, useKeymaps } = await import('./use-keymaps')
    await loadKeymaps()

    const a = useKeymaps()
    const b = useKeymaps()
    expect(a.keymaps).toBe(b.keymaps)
    expect(a.keymaps.value).toEqual(fixture)
  })
})

describe('isEditableTarget', () => {
  it('flags <input> elements', () => {
    const el = document.createElement('input')
    expect(isEditableTarget(el)).toBe(true)
  })

  it('flags <textarea> elements', () => {
    const el = document.createElement('textarea')
    expect(isEditableTarget(el)).toBe(true)
  })

  it('flags contenteditable elements', () => {
    const el = document.createElement('div')
    el.setAttribute('contenteditable', 'true')
    expect(isEditableTarget(el)).toBe(true)
  })

  it('rejects non-editable elements', () => {
    const el = document.createElement('div')
    expect(isEditableTarget(el)).toBe(false)
  })

  it('rejects null / non-element targets', () => {
    expect(isEditableTarget(null)).toBe(false)
  })
})
