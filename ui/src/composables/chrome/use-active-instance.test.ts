import { beforeEach, describe, expect, it, vi } from 'vitest'

import { InstanceState, TauriEvent } from '@ipc'

import { ToastTone } from '@components'

type Handler = (payload: { payload: unknown }) => void

const { handlers, unlisten } = vi.hoisted(() => ({
  handlers: new Map<string, Handler>(),
  unlisten: vi.fn()
}))

vi.mock('@ipc/bridge', async () => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: vi.fn(),
  listen: (event: string, cb: Handler) => {
    handlers.set(event, cb)

    return Promise.resolve(unlisten)
  }
}))

import {
  __resetActiveInstanceForTests,
  recordInstanceState,
  startActiveInstance,
  useActiveInstance
} from '@composables'
import { clearToasts, useToasts } from '@composables'

function emit(event: string, payload: unknown) {
  const cb = handlers.get(event)
  if (!cb) {
    throw new Error(`no listener registered for ${event}`)
  }
  cb({ payload })
}

beforeEach(() => {
  handlers.clear()
  unlisten.mockReset()
  __resetActiveInstanceForTests()
  clearToasts()
})

describe('useActiveInstance', () => {
  it('first setIfUnset populates the active id', () => {
    const { id, setIfUnset } = useActiveInstance()
    expect(id.value).toBeUndefined()
    setIfUnset('inst-a')
    expect(id.value).toBe('inst-a')
  })

  it('second setIfUnset does not overwrite an existing id', () => {
    const { id, setIfUnset } = useActiveInstance()
    setIfUnset('inst-a')
    setIfUnset('inst-b')
    expect(id.value).toBe('inst-a')
  })

  it('set always overwrites even when the id is already populated', () => {
    const { id, set, setIfUnset } = useActiveInstance()
    setIfUnset('inst-a')
    set('inst-b')
    expect(id.value).toBe('inst-b')
  })

  it('shares module-scoped state across callers', () => {
    const a = useActiveInstance()
    const b = useActiveInstance()
    a.set('inst-shared')
    expect(b.id.value).toBe('inst-shared')
  })

  it('clear() drops the active id', () => {
    const { id, set, clear } = useActiveInstance()
    set('inst-a')
    clear()
    expect(id.value).toBeUndefined()
  })

  it('subscribes to acp:instances-focused + acp:instances-changed', async () => {
    await startActiveInstance()
    expect([...handlers.keys()].sort()).toEqual([TauriEvent.AcpInstancesChanged, TauriEvent.AcpInstancesFocused])
  })

  it('routes acp:instances-focused payload onto the active id', async () => {
    await startActiveInstance()

    emit(TauriEvent.AcpInstancesFocused, { instanceId: 'inst-X' })
    expect(useActiveInstance().id.value).toBe('inst-X')

    emit(TauriEvent.AcpInstancesFocused, { instanceId: 'inst-Y' })
    expect(useActiveInstance().id.value).toBe('inst-Y')
  })

  it('clears the active id when the registry empties (instanceId omitted)', async () => {
    await startActiveInstance()

    emit(TauriEvent.AcpInstancesFocused, { instanceId: 'inst-A' })
    expect(useActiveInstance().id.value).toBe('inst-A')

    emit(TauriEvent.AcpInstancesFocused, {})
    expect(useActiveInstance().id.value).toBeUndefined()
  })

  it('toasts a warn tone when the previous instance had an Error state', async () => {
    await startActiveInstance()

    emit(TauriEvent.AcpInstancesFocused, { instanceId: 'inst-A' })
    recordInstanceState('inst-A', 'claude-code', InstanceState.Error)
    clearToasts()
    emit(TauriEvent.AcpInstancesFocused, { instanceId: 'inst-B' })

    const { entries } = useToasts()
    expect(entries.value).toHaveLength(1)
    expect(entries.value[0]?.tone).toBe(ToastTone.Warn)
    expect(entries.value[0]?.body).toContain('claude-code')
    expect(entries.value[0]?.body).toContain('exited')
  })

  it('toasts an ok tone when the previous instance had a clean Ended state', async () => {
    await startActiveInstance()

    emit(TauriEvent.AcpInstancesFocused, { instanceId: 'inst-A' })
    recordInstanceState('inst-A', 'claude-code', InstanceState.Ended)
    clearToasts()
    emit(TauriEvent.AcpInstancesFocused, { instanceId: 'inst-B' })

    const { entries } = useToasts()
    expect(entries.value[0]?.tone).toBe(ToastTone.Ok)
  })

  it('does not toast when manual set() flips the active id', () => {
    const { set } = useActiveInstance()
    set('inst-A')
    set('inst-B')

    const { entries } = useToasts()
    expect(entries.value).toHaveLength(0)
  })

  it('clears the active id when InstancesChanged drops it without a focused-id replacement', async () => {
    await startActiveInstance()

    emit(TauriEvent.AcpInstancesFocused, { instanceId: 'inst-A' })
    expect(useActiveInstance().id.value).toBe('inst-A')

    emit(TauriEvent.AcpInstancesChanged, { instanceIds: [] })
    expect(useActiveInstance().id.value).toBeUndefined()
  })

  it('startActiveInstance is idempotent', async () => {
    await startActiveInstance()
    const sizeAfterFirst = handlers.size
    await startActiveInstance()
    expect(handlers.size).toBe(sizeAfterFirst)
  })
})
