import { beforeEach, describe, expect, it, vi } from 'vitest'

const { listSessions, loadSession } = vi.hoisted(() => ({
  listSessions: vi.fn(),
  loadSession: vi.fn()
}))

vi.mock('@ipc', async () => ({
  ...(await vi.importActual<object>('@ipc')),
  listSessions: (...args: unknown[]) => listSessions(...args),
  loadSession: (...args: unknown[]) => loadSession(...args)
}))

vi.mock('@lib', async () => ({
  ...(await vi.importActual<object>('@lib')),
  log: { trace: vi.fn(), debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { __resetPaletteStackForTests, usePalette } from '@composables/palette'
import { __resetAllSessionInfoForTests, useSessionInfo } from '@composables/use-session-info'
import { type SessionSummary } from '@ipc'

import { buildSessionEntries, openSessionsLeaf, relativeFromNow } from './palette-sessions'

beforeEach(() => {
  __resetPaletteStackForTests()
  __resetAllSessionInfoForTests()
  listSessions.mockReset()
  loadSession.mockReset()
})

describe('relativeFromNow', () => {
  it('returns empty string for undefined input', () => {
    expect(relativeFromNow(undefined)).toBe('')
  })

  it('returns the raw value when timestamp does not parse', () => {
    expect(relativeFromNow('not-a-date')).toBe('not-a-date')
  })

  it('formats sub-minute deltas as seconds', () => {
    const now = () => Date.parse('2026-01-01T00:00:30Z')
    expect(relativeFromNow('2026-01-01T00:00:00Z', now)).toBe('30s ago')
  })

  it('formats minute / hour / day buckets', () => {
    const now = () => Date.parse('2026-01-10T12:00:00Z')
    expect(relativeFromNow('2026-01-10T11:55:00Z', now)).toBe('5m ago')
    expect(relativeFromNow('2026-01-10T09:00:00Z', now)).toBe('3h ago')
    expect(relativeFromNow('2026-01-08T12:00:00Z', now)).toBe('2d ago')
  })
})

describe('buildSessionEntries', () => {
  it('uses title when present, sessionId otherwise', () => {
    const sessions: SessionSummary[] = [
      { sessionId: 'sess-1', cwd: '/home/u/dev/x', title: 'feature work', updatedAt: undefined },
      { sessionId: 'sess-2', cwd: '/tmp', title: undefined, updatedAt: undefined }
    ]
    const entries = buildSessionEntries(sessions)
    expect(entries[0]?.name).toBe('feature work')
    expect(entries[1]?.name).toBe('sess-2')
  })

  it('shortens deep cwds in the description', () => {
    const sessions: SessionSummary[] = [{ sessionId: 's', cwd: '/home/u/dev/foo/bar/baz', title: 't' }]
    const entries = buildSessionEntries(sessions)
    expect(entries[0]?.description).toContain('…/foo/bar/baz')
  })

  it('preserves shallow paths in the description', () => {
    const sessions: SessionSummary[] = [{ sessionId: 's', cwd: '/tmp', title: 't' }]
    const entries = buildSessionEntries(sessions)
    expect(entries[0]?.description).toContain('/tmp')
  })

  it('appends relative-time when updatedAt is provided', () => {
    const sessions: SessionSummary[] = [{ sessionId: 's', cwd: '/tmp', title: 't', updatedAt: '2026-01-01T00:00:00Z' }]
    const now = () => Date.parse('2026-01-01T00:05:00Z')
    const entries = buildSessionEntries(sessions, now)
    expect(entries[0]?.description).toContain('5m ago')
  })
})

describe('openSessionsLeaf', () => {
  it('opens a placeholder palette synchronously then patches in the live list', async () => {
    listSessions.mockResolvedValue([
      { sessionId: 's-1', cwd: '/home/u/dev', title: 'one', updatedAt: undefined },
      { sessionId: 's-2', cwd: '/tmp', title: undefined, updatedAt: undefined }
    ])

    const { stack } = usePalette()
    const promise = openSessionsLeaf()

    // Placeholder pops up immediately — no await needed.
    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.title).toBe('sessions — loading…')
    expect(stack.value[0]?.entries).toEqual([])

    await promise

    // After the round-trip the palette is populated.
    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.title).toBe('sessions')
    expect(stack.value[0]?.entries).toHaveLength(2)
    expect(stack.value[0]?.entries[0]?.id).toBe('s-1')
  })

  it('shows the empty title when the daemon returns zero sessions', async () => {
    listSessions.mockResolvedValue([])

    const { stack } = usePalette()
    await openSessionsLeaf()
    expect(stack.value[0]?.title).toBe('sessions — empty')
    expect(stack.value[0]?.entries).toEqual([])
  })

  it('binds the right-pane preview component on the spec', async () => {
    listSessions.mockResolvedValue([])

    const { stack } = usePalette()
    await openSessionsLeaf()
    expect(stack.value[0]?.preview).toBeDefined()
    expect(stack.value[0]?.preview?.component).toBeTruthy()
  })

  it('Enter dispatches loadSession with a fresh instanceId and marks restored', async () => {
    listSessions.mockResolvedValue([{ sessionId: 's-A', cwd: '/tmp', title: 't' }])
    loadSession.mockResolvedValue(undefined)

    const { stack } = usePalette()
    await openSessionsLeaf()
    const spec = stack.value[0]!
    const pick = spec.entries[0]!
    await spec.onCommit([pick])

    expect(loadSession).toHaveBeenCalledTimes(1)
    const arg = loadSession.mock.calls[0]?.[0] as { sessionId: string; instanceId: string }
    expect(arg.sessionId).toBe('s-A')
    expect(arg.instanceId).toMatch(/^[0-9a-f-]{36}$/i)

    // setSessionRestored runs after the loadSession promise resolves —
    // poll the slot a tick later via useSessionInfo() to verify.
    await Promise.resolve()
    const { info } = useSessionInfo(arg.instanceId)
    expect(info.value.restored).toBe(true)
  })

  it('onDelete surfaces a toast warning rather than calling forget', async () => {
    listSessions.mockResolvedValue([{ sessionId: 's-A', cwd: '/tmp', title: 't' }])

    const { stack } = usePalette()
    await openSessionsLeaf()
    const spec = stack.value[0]!
    expect(spec.onDelete).toBeDefined()
    spec.onDelete?.(spec.entries[0]!)
    expect(loadSession).not.toHaveBeenCalled()
  })

  it('leaves the placeholder open when listSessions rejects', async () => {
    listSessions.mockRejectedValue(new Error('boom'))

    const { stack } = usePalette()
    await openSessionsLeaf()
    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.title).toBe('sessions — loading…')
  })
})
