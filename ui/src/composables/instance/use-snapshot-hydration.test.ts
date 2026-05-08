import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { mount, flushPromises } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { computed, defineComponent, h, nextTick, ref } from 'vue'

import { useSessionInfo } from './use-session-info'
import { useSnapshotHydration } from './use-snapshot-hydration'
import { useTurns } from './use-turns'
import { __resetActiveInstanceForTests } from '../chrome/use-active-instance'
import { TauriCommand, type MetaSnapshot, type TurnSnapshot } from '@ipc'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: () => Promise.resolve(() => {})
}))

function buildClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false, gcTime: 60_000, staleTime: 0
      }
    }
  })
}

/**
 * Mount `useSnapshotHydration` inside a host component so the
 * composable's internal `watch` registers under a real component
 * scope. We expose `setInstance` so tests can flip the active id.
 */
function mountHydration(opts: { instanceId: string | undefined; client: QueryClient }) {
  const idRef = ref<string | undefined>(opts.instanceId)
  const Host = defineComponent({
    setup() {
      const c = computed(() => idRef.value)

      useSnapshotHydration(c)

      return () => h('div')
    }
  })

  const wrapper = mount(Host, {
    global: { plugins: [[VueQueryPlugin, { queryClient: opts.client }]] }
  })

  return {
    wrapper,
    setInstance: (next: string | undefined): void => {
      idRef.value = next
    }
  }
}

function metaSnapshotFixture(overrides: Partial<MetaSnapshot> = {}): MetaSnapshot {
  return {
    profileId: 'personal/claude/opus',
    sessionId: 's-1',
    cwd: '/home/cenk/notes',
    currentModeId: 'plan',
    currentModelId: 'claude-opus-4',
    availableModes: [
      {
        id: 'plan', name: 'Plan'
      },
      {
        id: 'edit', name: 'Edit'
      }
    ],
    availableModels: [
      {
        id: 'claude-opus-4', name: 'Opus'
      },
      {
        id: 'claude-sonnet-4', name: 'Sonnet'
      }
    ],
    configOptions: [],
    mcpsCount: 3,
    pendingPermissions: [],
    usage: {
      used: 0, size: 0
    },
    turns: [],
    ...overrides
  }
}

function turnFixture(overrides: Partial<TurnSnapshot> = {}): TurnSnapshot {
  return {
    id: 't-1',
    sessionId: 's-1',
    startedAtMs: 1_700_000_000_000,
    ...overrides
  }
}

beforeEach(() => {
  invoke.mockReset()
  __resetActiveInstanceForTests()
  // Wipe useTurns + useSessionInfo module state between tests so
  // hydration doesn't see stale records from a previous run.
  // No public reset, so flip via the same mutation surface the
  // tests-under-test use.
})

afterEach(() => {
  invoke.mockReset()
})

describe('useSnapshotHydration — session-info hydration', () => {
  it('pushes cwd / mode / model / profile / mcps / available-lists from MetaSnapshot', async() => {
    const meta = metaSnapshotFixture()

    invoke.mockImplementation((command) => {
      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve(meta)
      }

      if (command === TauriCommand.ProfilesList) {
        return Promise.resolve({ profiles: [] })
      }

      return Promise.reject(new Error(`unexpected: ${String(command)}`))
    })
    const client = buildClient()

    mountHydration({ instanceId: 'i-1', client })
    await flushPromises()
    await flushPromises()

    const info = useSessionInfo('i-1').info.value

    expect(info.cwd).toBe('/home/cenk/notes')
    expect(info.mode).toBe('plan')
    expect(info.model).toBe('claude-opus-4')
    expect(info.profileId).toBe('personal/claude/opus')
    expect(info.mcpsCount).toBe(3)
    expect(info.availableModes).toHaveLength(2)
    expect(info.availableModes[0]?.id).toBe('plan')
    expect(info.availableModels).toHaveLength(2)
    expect(info.availableModels[0]?.id).toBe('claude-opus-4')
  })

  it('skips fields the snapshot omits — partial meta is OK', async() => {
    const meta: MetaSnapshot = {
      profileId: undefined,
      sessionId: 's-2',
      cwd: undefined,
      currentModeId: undefined,
      currentModelId: undefined,
      availableModes: [],
      availableModels: [],
      configOptions: [],
      mcpsCount: 0,
      pendingPermissions: [],
      usage: {
        used: 0, size: 0
      },
      turns: []
    }

    invoke.mockImplementation((command) => {
      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve(meta)
      }

      if (command === TauriCommand.ProfilesList) {
        return Promise.resolve({ profiles: [] })
      }

      return Promise.reject(new Error(`unexpected: ${String(command)}`))
    })
    const client = buildClient()

    mountHydration({ instanceId: 'i-2', client })
    await flushPromises()
    await flushPromises()

    const info = useSessionInfo('i-2').info.value

    expect(info.cwd).toBeUndefined()
    expect(info.mode).toBeUndefined()
    expect(info.model).toBeUndefined()
    expect(info.profileId).toBeUndefined()
    expect(info.mcpsCount).toBe(0)
  })

  it('hydrates configOptions when the daemon ships any', async() => {
    const meta = metaSnapshotFixture({
      configOptions: [
        {
          id: 'effort',
          name: 'Effort',
          currentValue: 'high',
          options: [
            {
              value: 'low', name: 'Low'
            },
            {
              value: 'high', name: 'High'
            }
          ]
        }
      ]
    })

    invoke.mockImplementation((command) => {
      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve(meta)
      }

      if (command === TauriCommand.ProfilesList) {
        return Promise.resolve({ profiles: [] })
      }

      return Promise.reject(new Error(`unexpected: ${String(command)}`))
    })
    const client = buildClient()

    mountHydration({ instanceId: 'i-3', client })
    await flushPromises()
    await flushPromises()

    const info = useSessionInfo('i-3').info.value

    expect(info.configOptions).toHaveLength(1)
    expect(info.configOptions[0]?.id).toBe('effort')
    expect(info.configOptions[0]?.currentValue).toBe('high')
  })
})

describe('useSnapshotHydration — turns replay', () => {
  it('replays turns into useTurns with a single dedup-set per instance', async() => {
    const meta = metaSnapshotFixture({
      turns: [
        turnFixture({
          id: 't-a', startedAtMs: 1000, endedAtMs: 2000, stopReason: 'end_turn'
        }),
        turnFixture({
          id: 't-b', startedAtMs: 2000, endedAtMs: 3000, stopReason: 'end_turn'
        }),
        turnFixture({
          id: 't-c', startedAtMs: 3000 // mid-flight
        })
      ]
    })

    invoke.mockImplementation((command) => {
      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve(meta)
      }

      if (command === TauriCommand.ProfilesList) {
        return Promise.resolve({ profiles: [] })
      }

      return Promise.reject(new Error(`unexpected: ${String(command)}`))
    })
    const client = buildClient()

    mountHydration({ instanceId: 'i-turns', client })
    await flushPromises()
    await flushPromises()

    const turns = useTurns('i-turns').turns.value

    expect(turns).toHaveLength(3)
    expect(turns[0]?.id).toBe('t-a')
    expect(turns[0]?.endedAtMs).toBe(2000)
    expect(turns[1]?.id).toBe('t-b')
    expect(turns[2]?.id).toBe('t-c')
    expect(turns[2]?.endedAtMs).toBeUndefined()
    // Only the still-open turn drives `openTurnId`.
    expect(useTurns('i-turns').openTurnId.value).toBe('t-c')
  })

  it('replaying the same meta data twice does not duplicate records', async() => {
    const meta = metaSnapshotFixture({
      turns: [turnFixture({
        id: 't-1', startedAtMs: 1000, endedAtMs: 2000
      })]
    })
    const client = buildClient()

    invoke.mockImplementation((command) => {
      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve(meta)
      }

      if (command === TauriCommand.ProfilesList) {
        return Promise.resolve({ profiles: [] })
      }

      return Promise.reject(new Error(`unexpected: ${String(command)}`))
    })
    mountHydration({ instanceId: 'i-dedup', client })
    await flushPromises()
    await flushPromises()
    // Force a refetch by invalidating the meta query cache.
    await client.invalidateQueries({ queryKey: ['snapshot-meta', 'i-dedup'] })
    await flushPromises()
    await flushPromises()

    const turns = useTurns('i-dedup').turns.value

    expect(turns).toHaveLength(1)
  })
})

describe('useSnapshotHydration — instance flip', () => {
  it('replays the new instance\'s meta when the watched id changes', async() => {
    const metaA = metaSnapshotFixture({
      cwd: '/repo-a', mcpsCount: 1
    })
    const metaB = metaSnapshotFixture({
      cwd: '/repo-b', mcpsCount: 2
    })

    invoke.mockImplementation((command, args) => {
      if (command !== TauriCommand.InstanceSnapshotMeta) {
        return Promise.reject(new Error(`unexpected: ${command}`))
      }
      const id = (args as { instanceId?: string } | undefined)?.instanceId

      return Promise.resolve(id === 'i-A' ? metaA : metaB)
    })
    const client = buildClient()
    const { setInstance } = mountHydration({ instanceId: 'i-A', client })

    await flushPromises()
    await flushPromises()
    expect(useSessionInfo('i-A').info.value.cwd).toBe('/repo-a')

    setInstance('i-B')
    await nextTick()
    await flushPromises()
    await flushPromises()
    expect(useSessionInfo('i-B').info.value.cwd).toBe('/repo-b')
  })
})
