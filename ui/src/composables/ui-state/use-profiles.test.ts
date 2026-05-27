import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'

import { __resetUseProfilesForTests, applyBootProfiles } from './use-profiles'
import { useProfiles } from '@composables'
import { TauriCommand, TauriEvent } from '@ipc'

const { invokeMock, listenMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
  listeners: new Map<string, (payload: { payload: unknown }) => void>()
}))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args),
  listen: (event: string, cb: (payload: { payload: unknown }) => void) => {
    listeners.set(event, cb)
    listenMock(event, cb)

    return Promise.resolve(() => listeners.delete(event))
  }
}))

interface ProfileFixture {
  id: string
  agent: string
  isDefault: boolean
}

function wireRpc(profiles: ProfileFixture[], selected: string | null): void {
  invokeMock.mockImplementation((command: string) => {
    if (command === TauriCommand.ProfilesList) {
      return Promise.resolve({ profiles })
    }

    if (command === TauriCommand.ProfileGet) {
      return Promise.resolve(selected)
    }

    if (command === TauriCommand.ProfileSet) {
      return Promise.resolve({ profileId: 'unused' })
    }

    return Promise.reject(new Error(`unexpected: ${command}`))
  })
}

beforeEach(() => {
  invokeMock.mockReset()
  listenMock.mockReset()
  listeners.clear()
  __resetUseProfilesForTests()
})

function host() {
  return defineComponent({
    setup(_, { expose }) {
      const composable = useProfiles()

      expose(composable)

      return () =>
        h('div', [h('span', { 'data-testid': 'selected' }, composable.selected.value ?? 'none'), h('span', { 'data-testid': 'count' }, String(composable.profiles.value.length))])
    }
  })
}

async function flushAsync(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
}

describe('useProfiles', () => {
  it('fetches profiles + the daemon-selected default on mount', async() => {
    wireRpc(
      [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        },
        {
          id: 'strict',
          agent: 'claude-code',
          isDefault: false
        }
      ],
      'ask'
    )

    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-testid="count"]').text()).toBe('2')
    expect(wrapper.get('[data-testid="selected"]').text()).toBe('ask')
  })

  it('renders "none" when daemon has no selected profile', async() => {
    wireRpc(
      [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: false
        }
      ],
      null
    )

    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-testid="selected"]').text()).toBe('none')
  })

  it('refresh() re-fetches and updates the reactive list', async() => {
    wireRpc(
      [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        }
      ],
      'ask'
    )
    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-testid="count"]').text()).toBe('1')

    wireRpc(
      [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        },
        {
          id: 'new-one',
          agent: 'codex',
          isDefault: false
        }
      ],
      'ask'
    )
    await (wrapper.vm as unknown as ReturnType<typeof useProfiles>).refresh()
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-testid="count"]').text()).toBe('2')
  })

  it('select() invokes profile_set on the daemon', async() => {
    wireRpc(
      [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        },
        {
          id: 'strict',
          agent: 'claude-code',
          isDefault: false
        }
      ],
      'ask'
    )
    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()
    await (wrapper.vm as unknown as ReturnType<typeof useProfiles>).select('strict')

    expect(invokeMock).toHaveBeenCalledWith(TauriCommand.ProfileSet, { profileId: 'strict' })
  })

  it('select() ignores ids not in the current list (no invoke)', async() => {
    wireRpc(
      [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        }
      ],
      'ask'
    )
    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()
    invokeMock.mockClear()
    await (wrapper.vm as unknown as ReturnType<typeof useProfiles>).select('ghost')

    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('acp:profile-changed event updates selected.value', async() => {
    wireRpc(
      [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        },
        {
          id: 'strict',
          agent: 'claude-code',
          isDefault: false
        }
      ],
      'ask'
    )
    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-testid="selected"]').text()).toBe('ask')

    const cb = listeners.get(TauriEvent.AcpProfileChanged)

    expect(cb).toBeDefined()
    cb!({ payload: { profileId: 'strict' } })
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-testid="selected"]').text()).toBe('strict')
  })

  it('applyBootProfiles seeds the singleton without invoking', async() => {
    applyBootProfiles(
      [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        },
        {
          id: 'strict',
          agent: 'claude-code',
          isDefault: false
        }
      ],
      'strict'
    )
    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-testid="count"]').text()).toBe('2')
    expect(wrapper.get('[data-testid="selected"]').text()).toBe('strict')
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('applyBootProfiles wires the acp:profile-changed listener (palette pick propagates)', async() => {
    // Regression: applyBootProfiles used to set initialised=true
    // without calling subscribe(), so the daemon's
    // `acp:profile-changed` event never reached `selected.value`.
    // Captains who picked a profile via the palette saw the daemon
    // accept the change (toast green) but the header pill + idle
    // session list stayed stuck on the boot-time value until reload.
    applyBootProfiles(
      [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        },
        {
          id: 'strict',
          agent: 'claude-code',
          isDefault: false
        }
      ],
      'ask'
    )
    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-testid="selected"]').text()).toBe('ask')

    const cb = listeners.get(TauriEvent.AcpProfileChanged)

    expect(cb, 'applyBootProfiles must subscribe to acp:profile-changed').toBeDefined()
    cb!({ payload: { profileId: 'strict' } })
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-testid="selected"]').text()).toBe('strict')
  })
})
