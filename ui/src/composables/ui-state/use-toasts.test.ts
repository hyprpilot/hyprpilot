import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ToastTone } from '@components'

import { clearToasts, dismissToast, pushToast, useToasts } from './use-toasts'

beforeEach(() => {
  vi.useFakeTimers()
  clearToasts()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useToasts', () => {
  it('records pushed entries in FIFO order with cap of 3', () => {
    pushToast(ToastTone.Ok, 'first')
    pushToast(ToastTone.Ok, 'second')
    pushToast(ToastTone.Ok, 'third')
    pushToast(ToastTone.Err, 'fourth')

    const { entries } = useToasts()
    expect(entries.value).toHaveLength(3)
    expect(entries.value.map((t) => t.body)).toEqual(['second', 'third', 'fourth'])
  })

  it('exposes the queue via `entries` (head is the visible toast)', () => {
    pushToast(ToastTone.Warn, 'now showing')
    pushToast(ToastTone.Ok, 'queued')

    const { entries } = useToasts()
    expect(entries.value[0]?.body).toBe('now showing')
    expect(entries.value).toHaveLength(2)
  })

  it('dismissToast removes the entry by id', () => {
    const id = pushToast(ToastTone.Warn, 'remove me')
    pushToast(ToastTone.Ok, 'keep me')

    dismissToast(id)

    const { entries } = useToasts()
    expect(entries.value).toHaveLength(1)
    expect(entries.value[0]?.body).toBe('keep me')
  })

  it('auto-dismisses entries after the default duration', () => {
    pushToast(ToastTone.Ok, 'auto')

    expect(useToasts().entries.value).toHaveLength(1)

    vi.advanceTimersByTime(4000)
    expect(useToasts().entries.value).toHaveLength(0)
  })

  it('honors custom duration for auto-dismiss', () => {
    pushToast(ToastTone.Warn, 'custom', { durationMs: 2000 })

    vi.advanceTimersByTime(1999)
    expect(useToasts().entries.value).toHaveLength(1)

    vi.advanceTimersByTime(1)
    expect(useToasts().entries.value).toHaveLength(0)
  })

  it('clearToasts empties the audit log', () => {
    pushToast(ToastTone.Ok, 'x')
    pushToast(ToastTone.Warn, 'y')
    clearToasts()

    expect(useToasts().entries.value).toHaveLength(0)
  })

  it('accepts a component+props body for richer toast contents', () => {
    const SampleBody = { template: '<span>x</span>' }
    pushToast(ToastTone.Warn, { component: SampleBody, props: { foo: 1 } })

    const head = useToasts().entries.value[0]
    expect(head?.body).toEqual({ component: SampleBody, props: { foo: 1 } })
  })
})
