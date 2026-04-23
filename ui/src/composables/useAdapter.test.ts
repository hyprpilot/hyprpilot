import { mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useAdapter, EventKind, SessionState } from '@composables/useAdapter'

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
        h('div', [
          h('span', { 'data-testid': 'transcript' }, JSON.stringify(agent.transcript)),
          h('span', { 'data-testid': 'state' }, agent.state.value?.state ?? 'none'),
          h('span', { 'data-testid': 'permission' }, agent.lastPermission.value?.session_id ?? 'none')
        ])
    }
  })
}

async function flushAsyncMounted(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
}

describe('useAdapter', () => {
  it('subscribes to every acp:* event channel on bind', async () => {
    const wrapper = mount(host())
    await flushAsyncMounted()

    const events = listen.mock.calls.map((c) => c[0])
    expect(events).toEqual(expect.arrayContaining(['acp:transcript', 'acp:session-state', 'acp:permission-request']))
    wrapper.unmount()
  })

  it('appends transcript payloads and reflects state transitions', async () => {
    const wrapper = mount(host())
    await flushAsyncMounted()

    const cbFor = (name: string) => {
      const entry = listen.mock.calls.find((c) => c[0] === name)

      return entry![1] as (payload: { payload: unknown }) => void
    }

    cbFor('acp:transcript')({
      payload: { kind: EventKind.Transcript, agent_id: 'a', session_id: 's-1', update: { kind: 'msg' } }
    })
    cbFor('acp:session-state')({
      payload: { kind: EventKind.State, agent_id: 'a', session_id: 's-1', state: SessionState.Running }
    })
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-testid="transcript"]').text()).toContain('s-1')
    expect(wrapper.get('[data-testid="state"]').text()).toBe('running')
    wrapper.unmount()
  })

  it('unsubscribes every channel on unmount', async () => {
    const wrapper = mount(host())
    await flushAsyncMounted()

    wrapper.unmount()

    expect(unlisten).toHaveBeenCalledTimes(3)
  })
})
