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

import { QueryClient } from '@tanstack/vue-query'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetActiveInstanceForTests, useActiveInstance } from './use-active-instance'
import { applyBootSnapshot } from './use-boot-snapshot'
import { TauriCommand } from '@ipc'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
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

describe('applyBootSnapshot — chat-cache hydration', () => {
  it('seeds the TanStack chat cache for every instance shipped in snap.chats', async() => {
    const chatPage = (instanceId: string, count: number) => ({
      items: Array.from({ length: count }, (_, i) => ({
        seq: i + 1,
        turnId: 't',
        item: { kind: 'user_prompt', text: `${instanceId}-msg-${i + 1}` }
      })),
      oldestSeq: 1,
      latestSeq: count,
      hasMore: false
    })

    invokeMock.mockResolvedValueOnce(
      snapshotFixture({
        instances: {
          instances: [
            { instanceId: 'i-a', agentId: 'claude-code' },
            { instanceId: 'i-b', agentId: 'claude-code' }
          ]
        },
        chats: {
          'i-a': chatPage('i-a', 42),
          'i-b': chatPage('i-b', 7)
        }
      })
    )
    const client = new QueryClient()

    await applyBootSnapshot(client)

    const cachedA = client.getQueryData(['snapshot-chat', 'i-a']) as { pages: { items: unknown[] }[] }
    const cachedB = client.getQueryData(['snapshot-chat', 'i-b']) as { pages: { items: unknown[] }[] }

    expect(cachedA?.pages[0]?.items).toHaveLength(42)
    expect(cachedB?.pages[0]?.items).toHaveLength(7)
  })

  it('merges boot chat with live cache instead of overwriting early events', async() => {
    invokeMock.mockResolvedValueOnce(
      snapshotFixture({
        instances: {
          instances: [{ instanceId: 'i-live', agentId: 'claude-code' }]
        },
        chats: {
          'i-live': {
            items: [
              {
                seq: 2,
                turnId: 't',
                item: { kind: 'agent_text', text: 'second' }
              }
            ],
            oldestSeq: 2,
            latestSeq: 2,
            hasMore: false
          }
        }
      })
    )
    const client = new QueryClient()

    client.setQueryData(['snapshot-chat', 'i-live'], {
      pages: [
        {
          items: [
            {
              seq: 1,
              turnId: 't',
              item: { kind: 'user_prompt', text: 'first' }
            }
          ],
          oldestSeq: 1,
          latestSeq: 1,
          hasMore: false
        }
      ],
      pageParams: [undefined]
    })

    await applyBootSnapshot(client)

    const cached = client.getQueryData(['snapshot-chat', 'i-live']) as { pages: { items: { seq: number }[]; oldestSeq?: number; latestSeq?: number }[] }

    expect(cached.pages[0]?.items.map((item) => item.seq)).toEqual([1, 2])
    expect(cached.pages[0]?.oldestSeq).toBe(1)
    expect(cached.pages[0]?.latestSeq).toBe(2)
  })

  it('no-ops when snap.chats is absent (older daemon)', async() => {
    invokeMock.mockResolvedValueOnce(
      snapshotFixture({
        instances: {
          instances: [{ instanceId: 'i-x', agentId: 'claude-code' }]
        }
        // no `chats` key — older daemon shape
      })
    )
    const client = new QueryClient()

    await expect(applyBootSnapshot(client)).resolves.toBe(true)
    expect(client.getQueryData(['snapshot-chat', 'i-x'])).toBeUndefined()
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
