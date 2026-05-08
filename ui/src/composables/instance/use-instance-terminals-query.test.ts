import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, ref, type Ref } from 'vue'

import { useInstanceTerminalsQuery } from './use-instance-terminals-query'
import { TauriCommand, type TerminalsSnapshot } from '@ipc'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: vi.fn()
}))

function fixture(): TerminalsSnapshot {
  return {
    terminals: {
      'term-1': {
        stdout: 'ok\n', running: false, exitCode: 0
      }
    }
  }
}

function buildClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false, gcTime: 0, staleTime: 0
      }
    }
  })
}

interface Probe {
  data: Ref<TerminalsSnapshot | undefined>
  isPending: Ref<boolean>
  isError: Ref<boolean>
  error: Ref<Error | null>
}

function mountWith(idRef: Ref<string | undefined>): { probe: Probe; unmount: () => void } {
  const probe: Partial<Probe> = {}
  const TestComponent = defineComponent({
    setup() {
      const id = ref(idRef.value)

      Object.assign(probe, useInstanceTerminalsQuery(id as unknown as Ref<string | undefined> as never))

      return () => h('div')
    }
  })
  const queryClient = buildClient()
  const wrapper = mount(TestComponent, {
    global: { plugins: [[VueQueryPlugin, { queryClient }]] }
  })

  return { probe: probe as Probe, unmount: () => wrapper.unmount() }
}

beforeEach(() => {
  invoke.mockReset()
})

describe('useInstanceTerminalsQuery', () => {
  it('returns data when invoke resolves', async() => {
    invoke.mockResolvedValue(fixture())
    const id = ref<string | undefined>('i-1')
    const { probe, unmount } = mountWith(id)

    await flushPromises()
    await flushPromises()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceSnapshotTerminals, { instanceId: 'i-1' })
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
