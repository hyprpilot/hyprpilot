import { mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useAdapter } from '@composables/useAdapter'

const unlisten = vi.fn()
const listen = vi.fn()
const invoke = vi.fn()

vi.mock('@ipc', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: (event: string, cb: (payload: { payload: unknown }) => void) => {
    listen(event, cb)

    return Promise.resolve(unlisten)
  }
}))

beforeEach(() => {
  unlisten.mockReset()
  listen.mockReset()
  invoke.mockReset()
})

function host() {
  return defineComponent({
    setup() {
      const agent = useAdapter()
      agent.bind()

      return () =>
        h('div', [h('span', { 'data-testid': 'permission' }, agent.lastPermission.value?.session_id ?? 'none')])
    }
  })
}

async function flushAsyncMounted(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
}

describe('useAdapter', () => {
  it('subscribes to acp:permission-request on bind', async () => {
    const wrapper = mount(host())
    await flushAsyncMounted()

    const events = listen.mock.calls.map((c) => c[0])
    expect(events).toEqual(['acp:permission-request'])
    wrapper.unmount()
  })

  it('exposes the last permission payload seen on the listener', async () => {
    const wrapper = mount(host())
    await flushAsyncMounted()

    const entry = listen.mock.calls.find((c) => c[0] === 'acp:permission-request')
    const cb = entry![1] as (payload: { payload: unknown }) => void
    cb({ payload: { agent_id: 'a', session_id: 's-1', options: [{ option_id: 'allow', name: 'Allow', kind: 'y' }] } })
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-testid="permission"]').text()).toBe('s-1')
    wrapper.unmount()
  })

  it('unsubscribes on unmount', async () => {
    const wrapper = mount(host())
    await flushAsyncMounted()

    wrapper.unmount()

    expect(unlisten).toHaveBeenCalledTimes(1)
  })
})
