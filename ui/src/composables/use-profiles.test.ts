import { mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useProfiles } from '@composables/use-profiles'

const getProfiles = vi.fn()

vi.mock('@ipc', () => ({
  getProfiles: () => getProfiles()
}))

beforeEach(() => {
  getProfiles.mockReset()
  window.localStorage.clear()
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
  it('fetches profiles and selects the configured default on mount', async () => {
    getProfiles.mockResolvedValueOnce([
      { id: 'ask', agent: 'claude-code', has_prompt: false, is_default: true },
      { id: 'strict', agent: 'claude-code', has_prompt: true, is_default: false }
    ])

    const wrapper = mount(host())
    await flushAsync()
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-testid="count"]').text()).toBe('2')
    expect(wrapper.get('[data-testid="selected"]').text()).toBe('ask')
  })

  it('refresh() re-fetches and updates the reactive list', async () => {
    getProfiles.mockResolvedValueOnce([{ id: 'ask', agent: 'claude-code', has_prompt: false, is_default: true }])
    const wrapper = mount(host())
    await flushAsync()
    await wrapper.vm.$nextTick()
    expect(wrapper.get('[data-testid="count"]').text()).toBe('1')

    getProfiles.mockResolvedValueOnce([
      { id: 'ask', agent: 'claude-code', has_prompt: false, is_default: true },
      { id: 'new-one', agent: 'codex', has_prompt: false, is_default: false }
    ])
    await (wrapper.vm as unknown as ReturnType<typeof useProfiles>).refresh()
    await wrapper.vm.$nextTick()

    expect(wrapper.get('[data-testid="count"]').text()).toBe('2')
  })

  it('select() persists the id to localStorage and next mount restores it', async () => {
    getProfiles.mockResolvedValueOnce([
      { id: 'ask', agent: 'claude-code', has_prompt: false, is_default: true },
      { id: 'strict', agent: 'claude-code', has_prompt: true, is_default: false }
    ])
    const wrapper = mount(host())
    await flushAsync()
    await wrapper.vm.$nextTick()
    ;(wrapper.vm as unknown as ReturnType<typeof useProfiles>).select('strict')

    expect(window.localStorage.getItem('hyprpilot:last-profile')).toBe('strict')

    getProfiles.mockResolvedValueOnce([
      { id: 'ask', agent: 'claude-code', has_prompt: false, is_default: true },
      { id: 'strict', agent: 'claude-code', has_prompt: true, is_default: false }
    ])
    const next = mount(host())
    await flushAsync()
    await next.vm.$nextTick()
    expect(next.get('[data-testid="selected"]').text()).toBe('strict')
  })

  it('ignores select() for ids not in the current list', async () => {
    getProfiles.mockResolvedValueOnce([{ id: 'ask', agent: 'claude-code', has_prompt: false, is_default: true }])
    const wrapper = mount(host())
    await flushAsync()
    await wrapper.vm.$nextTick()
    ;(wrapper.vm as unknown as ReturnType<typeof useProfiles>).select('ghost')

    expect(wrapper.get('[data-testid="selected"]').text()).toBe('ask')
  })
})
