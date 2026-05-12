/**
 * Regression coverage for the boot-snapshot apply pipeline.
 *
 * The captain symptom that drove these tests: a remote browser
 * stuck on the loading screen forever. Root cause: a buggy daemon
 * build of `boot_snapshot` shipped JSON `null` for unset
 * `Option<String>` fields (`name`, `profileId`, etc.) instead of
 * omitting the keys, and the UI's `entry.name !== undefined &&
 * entry.name.length > 0` guard let `null` through to
 * `null.length` — uncaught `TypeError`, swallowed by `void
 * boot()`, `markBootDone()` never fired.
 *
 * Tests:
 *   - `applyBootSnapshot` survives a `null`-laden instance entry
 *     and still returns true (the loosely-typed `!= null` guard).
 *   - `applyBootSnapshot` calls each setter for present fields
 *     and skips them for null/undefined fields.
 *   - `setIfUnset(focusedId)` lands when no active instance is
 *     set, and stays put when one already is (preserves a
 *     captain's local choice).
 *   - Failure of `invoke(BootSnapshot)` returns `false` so
 *     `main.ts` can fall through to the granular loaders.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetActiveInstanceForTests, useActiveInstance } from './use-active-instance'
import { applyBootSnapshot } from './use-boot-snapshot'
import { TauriCommand } from '@ipc'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (...args: unknown[]) => invokeMock(...args)
}))

const setInstanceAgentMock = vi.fn()
const setInstanceProfileMock = vi.fn()
const setInstanceNameMock = vi.fn()
const pushCurrentModeUpdateMock = vi.fn()

vi.mock('../instance/use-session-info', () => ({
  setInstanceAgent: (...args: unknown[]) => setInstanceAgentMock(...args),
  setInstanceProfile: (...args: unknown[]) => setInstanceProfileMock(...args),
  setInstanceName: (...args: unknown[]) => setInstanceNameMock(...args),
  pushCurrentModeUpdate: (...args: unknown[]) => pushCurrentModeUpdateMock(...args)
}))

vi.mock('../instance/use-focus-prefetch', () => ({
  prefetchInstanceMeta: vi.fn().mockResolvedValue(undefined),
  prefetchInstanceChatFirstPage: vi.fn().mockResolvedValue(undefined)
}))

function snapshotFixture(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    theme: { surface: { default: '#000000' } },
    keymaps: {},
    windowState: { mode: 'anchor', anchorEdge: 'right' },
    daemonCwd: '~/tmp',
    completionConfig: {
      ripgrep: {
        auto: true,
        debounceMs: 250,
        minPrefix: 3
      }
    },
    agents: { agents: [] },
    profiles: { profiles: [] },
    instances: { instances: [] },
    ...overrides
  }
}

beforeEach(() => {
  invokeMock.mockReset()
  setInstanceAgentMock.mockReset()
  setInstanceProfileMock.mockReset()
  setInstanceNameMock.mockReset()
  pushCurrentModeUpdateMock.mockReset()
  __resetActiveInstanceForTests()
})

describe('applyBootSnapshot — null safety on instance entries', () => {
  it('survives a buggy daemon shipping `null` for unset Option fields', async() => {
    invokeMock.mockResolvedValueOnce(
      snapshotFixture({
        instances: {
          instances: [
            {
              instanceId: 'i-1',
              agentId: 'claude-code',
              // Pre-fix daemon shape: explicit null instead of omitted.
              // The original `entry.name !== undefined` guard let these
              // through and crashed at `null.length`.
              name: null,
              profileId: null,
              sessionId: null,
              mode: null
            }
          ]
        }
      })
    )

    // Must not throw. Pre-fix this would TypeError at `null.length`.
    const ok = await applyBootSnapshot()

    expect(ok).toBe(true)

    // Setters guarded by `!= null` should NOT have been called for the
    // null fields. agentId is truthy, so it lands.
    expect(setInstanceAgentMock).toHaveBeenCalledWith('i-1', 'claude-code')
    expect(setInstanceProfileMock).not.toHaveBeenCalled()
    expect(setInstanceNameMock).not.toHaveBeenCalled()
    expect(pushCurrentModeUpdateMock).not.toHaveBeenCalled()
  })

  it('seeds session-info from a fully populated instance entry', async() => {
    invokeMock.mockResolvedValueOnce(
      snapshotFixture({
        instances: {
          instances: [
            {
              instanceId: 'i-2',
              agentId: 'claude-code',
              name: 'alpha',
              profileId: 'personal/claude/opus',
              mode: 'plan'
            }
          ]
        }
      })
    )

    await applyBootSnapshot()

    expect(setInstanceAgentMock).toHaveBeenCalledWith('i-2', 'claude-code')
    expect(setInstanceProfileMock).toHaveBeenCalledWith('i-2', 'personal/claude/opus')
    expect(setInstanceNameMock).toHaveBeenCalledWith('i-2', 'alpha')
    expect(pushCurrentModeUpdateMock).toHaveBeenCalledWith('i-2', { currentModeId: 'plan' })
  })

  it('skips empty-string name even when present', async() => {
    invokeMock.mockResolvedValueOnce(
      snapshotFixture({
        instances: {
          instances: [
            {
              instanceId: 'i-3',
              agentId: 'claude-code',
              name: ''
            }
          ]
        }
      })
    )

    await applyBootSnapshot()
    expect(setInstanceNameMock).not.toHaveBeenCalled()
  })
})

describe('applyBootSnapshot — focused instance seeding', () => {
  it('lands focusedId on useActiveInstance when no choice is set', async() => {
    invokeMock.mockResolvedValueOnce(
      snapshotFixture({
        instances: {
          focusedId: 'i-focus',
          instances: [{ instanceId: 'i-focus', agentId: 'claude-code' }]
        }
      })
    )

    await applyBootSnapshot()
    expect(useActiveInstance().id.value).toBe('i-focus')
  })

  it('preserves a prior local choice via setIfUnset', async() => {
    useActiveInstance().set('local-choice')

    invokeMock.mockResolvedValueOnce(
      snapshotFixture({
        instances: {
          focusedId: 'daemon-focus',
          instances: [{ instanceId: 'daemon-focus', agentId: 'claude-code' }]
        }
      })
    )

    await applyBootSnapshot()
    expect(useActiveInstance().id.value).toBe('local-choice')
  })

  it('leaves activeInstance undefined when no focusedId is shipped', async() => {
    invokeMock.mockResolvedValueOnce(
      snapshotFixture({
        instances: { instances: [] }
      })
    )

    await applyBootSnapshot()
    expect(useActiveInstance().id.value).toBeUndefined()
  })
})

describe('applyBootSnapshot — failure modes', () => {
  it('returns false when invoke rejects (no Tauri host / older daemon)', async() => {
    invokeMock.mockRejectedValueOnce(new Error('host missing'))
    const ok = await applyBootSnapshot()

    expect(ok).toBe(false)
  })

  it('called the daemon with the boot_snapshot command', async() => {
    invokeMock.mockResolvedValueOnce(snapshotFixture())
    await applyBootSnapshot()
    expect(invokeMock).toHaveBeenCalledWith(TauriCommand.BootSnapshot)
  })
})
