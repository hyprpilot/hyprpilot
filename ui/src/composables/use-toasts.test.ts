import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ToastTone } from '@components/types'

import { clearToasts, dismissToast, pushToast, useToasts } from './use-toasts'

beforeEach(() => {
  vi.useFakeTimers()
  clearToasts()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useToasts', () => {
  it('push three toasts — all three appear in entries', () => {
    pushToast(ToastTone.Ok, 'a')
    pushToast(ToastTone.Warn, 'b')
    pushToast(ToastTone.Info, 'c')

    const { entries } = useToasts()
    expect(entries.value).toHaveLength(3)
    expect(entries.value.map((t) => t.message)).toEqual(['a', 'b', 'c'])
  })

  it('push a fourth toast — oldest (first) is dropped (FIFO cap at 3)', () => {
    pushToast(ToastTone.Ok, 'first')
    pushToast(ToastTone.Ok, 'second')
    pushToast(ToastTone.Ok, 'third')
    pushToast(ToastTone.Err, 'fourth')

    const { entries } = useToasts()
    expect(entries.value).toHaveLength(3)
    expect(entries.value.map((t) => t.message)).toEqual(['second', 'third', 'fourth'])
  })

  it('manual dismiss removes the entry by id', () => {
    const id = pushToast(ToastTone.Warn, 'remove me')
    pushToast(ToastTone.Ok, 'keep me')

    dismissToast(id)

    const { entries } = useToasts()
    expect(entries.value).toHaveLength(1)
    expect(entries.value[0]?.message).toBe('keep me')
  })

  it('entries auto-drop after the default 4000ms TTL', () => {
    pushToast(ToastTone.Ok, 'auto')

    const { entries } = useToasts()
    expect(entries.value).toHaveLength(1)

    vi.advanceTimersByTime(4000)
    expect(entries.value).toHaveLength(0)
  })

  it('custom ttlMs is honored — toast survives before expiry and drops after', () => {
    pushToast(ToastTone.Info, 'custom', 2000)

    const { entries } = useToasts()
    vi.advanceTimersByTime(1999)
    expect(entries.value).toHaveLength(1)

    vi.advanceTimersByTime(1)
    expect(entries.value).toHaveLength(0)
  })

  it('clearToasts empties the store', () => {
    pushToast(ToastTone.Ok, 'x')
    pushToast(ToastTone.Warn, 'y')
    clearToasts()

    const { entries } = useToasts()
    expect(entries.value).toHaveLength(0)
  })
})
