import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'

import { __resetUseProfilesForTests } from './use-profiles'
import { useProfiles } from '@composables'

const invokeMock = vi.fn()

// The composable calls `invoke(TauriCommand.ProfilesList)` and reads
// `r.profiles` off the response. Mock the bridge directly so the
// typed barrel imports keep their TauriCommand re-export visible.
vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (...args: unknown[]) => invokeMock(...args)
}))

function setProfiles(profiles: { id: string; agent: string; isDefault: boolean }[]): void {
  invokeMock.mockResolvedValueOnce({ profiles })
}

beforeEach(() => {
  invokeMock.mockReset()
  window.localStorage.clear()
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
  it('fetches profiles and selects the configured default on mount', async() => {
    setProfiles([
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
    ])

    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-testid="count"]').text()).toBe('2')
    expect(wrapper.get('[data-testid="selected"]').text()).toBe('ask')
  })

  it('refresh() re-fetches and updates the reactive list', async() => {
    setProfiles([
      {
        id: 'ask',
        agent: 'claude-code',
        isDefault: true
      }
    ])
    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-testid="count"]').text()).toBe('1')

    setProfiles([
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
    ])
    await (wrapper.vm as unknown as ReturnType<typeof useProfiles>).refresh()
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-testid="count"]').text()).toBe('2')
  })

  it('select() persists the id to localStorage and next mount restores it', async() => {
    setProfiles([
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
    ])
    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()
    ;(wrapper.vm as unknown as ReturnType<typeof useProfiles>).select('strict')

    expect(window.localStorage.getItem('hyprpilot:last-profile')).toBe('strict')

    // Simulate a fresh process — clear the singleton state so the
    // next mount triggers a real refresh and restores from localStorage.
    __resetUseProfilesForTests()
    setProfiles([
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
    ])
    const next = mount(host())

    await flushAsync()
    await next.vm.$nextTick()
    expect(next.get('[data-testid="selected"]').text()).toBe('strict')
  })

  it('ignores select() for ids not in the current list', async() => {
    setProfiles([
      {
        id: 'ask',
        agent: 'claude-code',
        isDefault: true
      }
    ])
    const wrapper = mount(host())

    await flushAsync()
    await wrapper.vm.$nextTick()
    ;(wrapper.vm as unknown as ReturnType<typeof useProfiles>).select('ghost')

    expect(wrapper.get('[data-testid="selected"]').text()).toBe('ask')
  })
})
