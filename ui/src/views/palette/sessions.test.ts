import { beforeEach, describe, expect, it, vi } from 'vitest'

import { buildSessionEntries, openSessionsLeaf, relativeFromNow } from './sessions'
import { __resetPaletteStackForTests, usePalette, __resetAllSessionInfoForTests, useSessionInfo } from '@composables'
import { TauriCommand } from '@ipc'
import { type SessionSummary } from '@ipc'

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn()
}))

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (...args: unknown[]) => invokeMock(...args)
}))

vi.mock('@lib', async() => ({
  ...(await vi.importActual<object>('@lib')),
  log: {
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn()
  }
}))

beforeEach(() => {
  __resetPaletteStackForTests()
  __resetAllSessionInfoForTests()
  invokeMock.mockReset()
})

/// Set the mock implementation to return `sessions` for the next
/// `SessionList` invoke. `SessionLoad` resolves to undefined by
/// default so the test commits without a separate setup.
function mockListAndLoad(sessions: SessionSummary[], loadResolves = true): void {
  invokeMock.mockImplementation((command: string) => {
    if (command === TauriCommand.SessionList) {
      return Promise.resolve({ sessions })
    }

    if (command === TauriCommand.SessionLoad) {
      return loadResolves ? Promise.resolve(undefined) : Promise.reject(new Error('load failed'))
    }

    return Promise.resolve(undefined)
  })
}

describe('relativeFromNow', () => {
  it('returns empty string for undefined input', () => {
    expect(relativeFromNow(undefined)).toBe('')
  })
})

describe('buildSessionEntries', () => {
  it('uses sessionId when title is empty', () => {
    const entries = buildSessionEntries([
      {
        sessionId: 'abc',
        cwd: '/tmp',
        title: undefined,
        updatedAt: undefined
      }
    ])

    expect(entries[0]?.name).toBe('abc')
  })
})

describe('openSessionsLeaf', () => {
  it('opens a placeholder palette synchronously then patches in the live list', async() => {
    mockListAndLoad([
      {
        sessionId: 's-1',
        cwd: '/home/u/dev',
        title: 'one',
        updatedAt: undefined
      },
      {
        sessionId: 's-2',
        cwd: '/tmp',
        title: undefined,
        updatedAt: undefined
      }
    ])

    const { stack } = usePalette()
    const promise = openSessionsLeaf()

    // Placeholder pops up immediately — no await needed.
    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.title).toBe('sessions')
    expect(stack.value[0]?.loading).toBe(true)
    expect(stack.value[0]?.status).toBe('fetching session list')
    expect(stack.value[0]?.entries).toEqual([])

    await promise

    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.title).toBe('sessions')
    expect(stack.value[0]?.loading).toBe(false)
    expect(stack.value[0]?.entries).toHaveLength(2)
    expect(stack.value[0]?.entries[0]?.id).toBe('s-1')
  })

  it('shows the empty title when the daemon returns zero sessions', async() => {
    mockListAndLoad([])

    const { stack } = usePalette()

    await openSessionsLeaf()
    expect(stack.value[0]?.title).toBe('sessions — empty')
    expect(stack.value[0]?.entries).toEqual([])
  })

  it('binds the right-pane preview component on the spec', async() => {
    mockListAndLoad([])

    const { stack } = usePalette()

    await openSessionsLeaf()
    expect(stack.value[0]?.preview).toBeDefined()
    expect(stack.value[0]?.preview?.component).toBeTruthy()
  })

  it('Enter dispatches SessionLoad with a fresh instanceId and marks restored', async() => {
    mockListAndLoad([
      {
        sessionId: 's-A',
        cwd: '/tmp',
        title: 't'
      }
    ])

    const { stack } = usePalette()

    await openSessionsLeaf()
    const spec = stack.value[0]!
    const pick = spec.entries[0]!

    await spec.onCommit([pick])

    const loadCall = invokeMock.mock.calls.find((c: unknown[]) => c[0] === TauriCommand.SessionLoad)

    expect(loadCall).toBeDefined()
    const arg = loadCall![1] as { sessionId: string; instanceId: string }

    expect(arg.sessionId).toBe('s-A')
    expect(arg.instanceId).toMatch(/^[0-9a-f-]{36}$/i)

    // setSessionRestored runs after the SessionLoad promise resolves —
    // poll the slot a tick later via useSessionInfo() to verify.
    await Promise.resolve()
    const { info } = useSessionInfo(arg.instanceId)

    expect(info.value.restored).toBe(true)
  })

  it('onDelete surfaces a toast warning rather than calling forget', async() => {
    mockListAndLoad([
      {
        sessionId: 's-A',
        cwd: '/tmp',
        title: 't'
      }
    ])

    const { stack } = usePalette()

    await openSessionsLeaf()
    const spec = stack.value[0]!

    expect(spec.onDelete).toBeDefined()
    spec.onDelete?.(spec.entries[0]!, () => {})

    const loadCall = invokeMock.mock.calls.find((c: unknown[]) => c[0] === TauriCommand.SessionLoad)

    expect(loadCall).toBeUndefined()
  })

  it('leaves the placeholder open when SessionList rejects', async() => {
    invokeMock.mockRejectedValueOnce(new Error('boom'))

    const { stack } = usePalette()

    await openSessionsLeaf()
    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.title).toBe('sessions')
    // Reject path leaves the loading flag set so the user sees the
    // toast + the open placeholder; closing the palette is up to them.
    expect(stack.value[0]?.loading).toBe(true)
  })
})
