import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, ref, type Ref } from 'vue'

import { useInstanceMetaQuery } from './use-instance-meta-query'
import { TauriCommand, type MetaSnapshot } from '@ipc'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: vi.fn()
}))

function fixture(): MetaSnapshot {
  return {
    cwd: '/tmp/proj',
    currentModeId: 'plan',
    currentModelId: 'sonnet',
    mcpsCount: 3,
    usage: { used: 0, size: 0 }
  }
}

function buildClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        gcTime: 0,
        staleTime: 0
      }
    }
  })
}

interface Probe {
  data: Ref<MetaSnapshot | undefined>
  isPending: Ref<boolean>
  isFetching: Ref<boolean>
  isError: Ref<boolean>
  error: Ref<Error | null>
}

function mountWith(idRef: Ref<string | undefined>): { probe: Probe; queryClient: QueryClient; unmount: () => void } {
  const probe: Partial<Probe> = {}
  const TestComponent = defineComponent({
    setup() {
      const id = ref(idRef.value)

      // Mirror the consumer pattern — `instanceId` is a ComputedRef
      // in production. Tests pass a plain ref; the composable's
      // `enabled` and `queryFn` only read `.value`.
      Object.assign(probe, useInstanceMetaQuery(id as unknown as Ref<string | undefined> as never))

      return () => h('div')
    }
  })
  const queryClient = buildClient()
  const wrapper = mount(TestComponent, {
    global: { plugins: [[VueQueryPlugin, { queryClient }]] }
  })

  return {
    probe: probe as Probe,
    queryClient,
    unmount: () => wrapper.unmount()
  }
}

beforeEach(() => {
  invoke.mockReset()
})

afterEach(() => {
  // Tests own teardown: every `mountWith` returns an `unmount` the
  // case calls before assertions land, mirroring `enableAutoUnmount`'s
  // afterEach hook from `vitest.setup.ts`.
})

describe('useInstanceMetaQuery', () => {
  it('returns data when invoke resolves', async() => {
    invoke.mockResolvedValue(fixture())
    const id = ref<string | undefined>('i-1')
    const { probe, unmount } = mountWith(id)

    await flushPromises()
    await flushPromises()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceSnapshotMeta, { instanceId: 'i-1' })
    expect(probe.data.value).toEqual(fixture())
    expect(probe.isError.value).toBe(false)
    unmount()
  })

  it('surfaces invoke error onto the query handle', async() => {
    const boom = new Error('mirror missing')

    invoke.mockRejectedValue(boom)
    const id = ref<string | undefined>('i-1')
    const { probe, unmount } = mountWith(id)

    await flushPromises()
    await flushPromises()
    await flushPromises()

    expect(probe.isError.value).toBe(true)
    expect(probe.error.value).toBe(boom)
    unmount()
  })

  it('is disabled when instanceId is undefined', async() => {
    invoke.mockResolvedValue(fixture())
    const id = ref<string | undefined>(undefined)
    const { probe, unmount } = mountWith(id)

    await flushPromises()
    await flushPromises()

    expect(invoke).not.toHaveBeenCalled()
    expect(probe.data.value).toBeUndefined()
    unmount()
  })
})
