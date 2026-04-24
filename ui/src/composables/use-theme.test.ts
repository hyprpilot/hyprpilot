import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { applyGtkFont } from './use-theme'

const invoke = vi.fn()

vi.mock('@ipc', async () => {
  const actual = await vi.importActual<typeof import('@ipc')>('@ipc')
  return {
    ...actual,
    invoke: (...args: unknown[]) => invoke(...args)
  }
})

beforeEach(() => {
  invoke.mockReset()
})

afterEach(() => {
  document.documentElement.style.removeProperty('--theme-font-sans')
})

describe('applyGtkFont', () => {
  it('overrides --theme-font-sans with the GTK family when get_gtk_font returns a font', async () => {
    invoke.mockResolvedValueOnce({ family: 'Inter', sizePt: 10 })

    await applyGtkFont()

    expect(document.documentElement.style.getPropertyValue('--theme-font-sans')).toBe(
      "'Inter', ui-sans-serif, system-ui, sans-serif"
    )
  })

  it('leaves --theme-font-sans untouched when the IPC returns null', async () => {
    invoke.mockResolvedValueOnce(null)

    await applyGtkFont()

    expect(document.documentElement.style.getPropertyValue('--theme-font-sans')).toBe('')
  })

  it('soft-fails without a Tauri host', async () => {
    invoke.mockRejectedValueOnce(new Error('tauri host missing'))

    await applyGtkFont()

    expect(document.documentElement.style.getPropertyValue('--theme-font-sans')).toBe('')
  })
})
