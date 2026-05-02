import { beforeEach, describe, expect, it, vi } from 'vitest'
import { computed, ref } from 'vue'

import { openInstancesLeaf } from './instances'
import { __resetPaletteStackForTests, usePalette } from '@composables'

const invokeMock = vi.fn()
const pushToastMock = vi.fn()

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args),
  listen: vi.fn()
}))

const activeInstanceRef = ref<string | undefined>(undefined)

vi.mock('@composables', async(importOriginal) => ({
  ...(await importOriginal<typeof import('@composables')>()),
  useActiveInstance: () => ({
    id: activeInstanceRef
  }),
  useHomeDir: () => ({
    homeDir: ref('/home/cenk')
  }),
  usePhase: () => ({
    phase: computed(() => 'idle')
  }),
  useQueue: () => ({
    items: computed(() => [])
  }),
  truncateCwd: (raw: string) => raw,
  useSessionInfo: () => ({
    info: computed(() => ({
      mode: undefined,
      cwd: '/home/cenk/dev',
      mcpsCount: 0,
      restored: false
    }))
  }),
  useTerminals: () => ({
    all: computed(() => [])
  }),
  pushToast: (...args: unknown[]) => pushToastMock(...args)
}))

beforeEach(() => {
  __resetPaletteStackForTests()
  invokeMock.mockReset()
  pushToastMock.mockReset()
  activeInstanceRef.value = undefined
})

describe('openInstancesLeaf', () => {
  it('renders an empty state when the daemon reports zero instances', async() => {
    invokeMock.mockResolvedValue({ instances: [] })

    await openInstancesLeaf()

    const { stack } = usePalette()

    expect(stack.value).toHaveLength(1)
    const spec = stack.value[0]

    expect(spec?.title).toBe('instances')
    expect(spec?.entries).toHaveLength(1)
    expect(spec?.entries[0]?.id).toBe('instances-empty')
    expect(spec?.entries[0]?.name).toBe('no live instances')
  })

  it('lists every live instance with agent + profile in the row name', async() => {
    invokeMock.mockResolvedValue({
      instances: [
        {
          agentId: 'claude-code',
          profileId: 'ask',
          instanceId: 'inst-A'
        },
        {
          agentId: 'codex',
          profileId: 'plan',
          instanceId: 'inst-B'
        }
      ]
    })

    await openInstancesLeaf()

    const { stack } = usePalette()

    expect(stack.value).toHaveLength(1)
    const entries = stack.value[0]?.entries ?? []

    expect(entries).toHaveLength(2)
    expect(entries[0]?.id).toBe('inst-A')
    expect(entries[0]?.name).toBe('claude-code · ask')
    expect(entries[1]?.name).toBe('codex · plan')
  })

  it('marks the active instance in its description prefix', async() => {
    activeInstanceRef.value = 'inst-A'
    invokeMock.mockResolvedValue({
      instances: [
        {
          agentId: 'claude-code',
          profileId: 'ask',
          instanceId: 'inst-A'
        }
      ]
    })

    await openInstancesLeaf()

    const { stack } = usePalette()
    const description = stack.value[0]?.entries[0]?.description ?? ''

    expect(description).toMatch(/active/)
  })

  it('falls back to "no-profile" when an instance has no profile id', async() => {
    invokeMock.mockResolvedValue({
      instances: [{ agentId: 'claude-code', instanceId: 'inst-A' }]
    })

    await openInstancesLeaf()

    const { stack } = usePalette()

    expect(stack.value[0]?.entries[0]?.name).toBe('claude-code · no-profile')
  })

  it('onCommit dispatches instances_focus to the daemon', async() => {
    invokeMock.mockResolvedValueOnce({
      instances: [
        {
          agentId: 'claude-code',
          profileId: 'ask',
          instanceId: 'inst-A'
        }
      ]
    })

    await openInstancesLeaf()

    invokeMock.mockResolvedValueOnce({ focusedId: 'inst-A' })

    const { stack } = usePalette()
    const spec = stack.value[0]

    spec?.onCommit([{ id: 'inst-A', name: 'claude-code · ask' }])
    await Promise.resolve()
    await Promise.resolve()

    const focusCalls = invokeMock.mock.calls.filter((c) => c[0] === 'instances_focus')

    expect(focusCalls).toHaveLength(1)
    expect(focusCalls[0]?.[1]).toEqual({ id: 'inst-A' })
  })

  it('onCommit on the empty-state row is a no-op', async() => {
    invokeMock.mockResolvedValue({ instances: [] })

    await openInstancesLeaf()

    const { stack } = usePalette()
    const spec = stack.value[0]

    spec?.onCommit([{ id: 'instances-empty', name: 'no live instances' }])
    await Promise.resolve()

    const focusCalls = invokeMock.mock.calls.filter((c) => c[0] === 'instances_focus')

    expect(focusCalls).toHaveLength(0)
  })

  it('onDelete dispatches instances_shutdown to the daemon', async() => {
    invokeMock.mockResolvedValueOnce({
      instances: [
        {
          agentId: 'claude-code',
          profileId: 'ask',
          instanceId: 'inst-A'
        }
      ]
    })

    await openInstancesLeaf()

    invokeMock.mockResolvedValueOnce({ id: 'inst-A' })

    const { stack } = usePalette()
    const spec = stack.value[0]

    spec?.onDelete?.({ id: 'inst-A', name: 'claude-code · ask' })
    await Promise.resolve()
    await Promise.resolve()

    const shutdownCalls = invokeMock.mock.calls.filter((c) => c[0] === 'instances_shutdown')

    expect(shutdownCalls).toHaveLength(1)
    expect(shutdownCalls[0]?.[1]).toEqual({ id: 'inst-A' })
  })

  it('onDelete on the empty-state row is a no-op', async() => {
    invokeMock.mockResolvedValue({ instances: [] })

    await openInstancesLeaf()

    const { stack } = usePalette()
    const spec = stack.value[0]

    spec?.onDelete?.({ id: 'instances-empty', name: 'no live instances' })
    await Promise.resolve()

    const shutdownCalls = invokeMock.mock.calls.filter((c) => c[0] === 'instances_shutdown')

    expect(shutdownCalls).toHaveLength(0)
  })

  it('toasts an error and renders the empty state when instances_list throws', async() => {
    invokeMock.mockRejectedValueOnce(new Error('socket down'))

    await openInstancesLeaf()

    expect(pushToastMock).toHaveBeenCalledTimes(1)
    expect(pushToastMock.mock.calls[0]?.[1]).toMatch(/instances list failed/)

    const { stack } = usePalette()

    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.entries[0]?.id).toBe('instances-empty')
  })
})
