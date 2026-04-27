import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { getSessionInfo } = vi.hoisted(() => ({
  getSessionInfo: vi.fn()
}))

vi.mock('@ipc', async () => ({
  ...(await vi.importActual<object>('@ipc')),
  getSessionInfo: (...args: unknown[]) => getSessionInfo(...args)
}))

import PaletteSessionsPreview from './PaletteSessionsPreview.vue'

beforeEach(() => {
  vi.useFakeTimers()
  getSessionInfo.mockReset()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('PaletteSessionsPreview.vue', () => {
  it('renders the empty-state message when no entry is bound', () => {
    const wrapper = mount(PaletteSessionsPreview, { props: {} })
    expect(wrapper.find('[data-testid="palette-sessions-preview-empty"]').exists()).toBe(true)
  })

  it('debounces the sessions/info round-trip by 200ms', async () => {
    getSessionInfo.mockResolvedValue({ id: 'A', cwd: '/tmp', agentId: 'a' })
    const wrapper = mount(PaletteSessionsPreview, {
      props: { entry: { id: 'A', name: 'a' } }
    })

    // Immediately after mount no IPC fired.
    expect(getSessionInfo).not.toHaveBeenCalled()

    // After 200ms the call lands.
    vi.advanceTimersByTime(200)
    expect(getSessionInfo).toHaveBeenCalledWith('A')

    await wrapper.unmount()
  })

  it('cancels the pending call when the entry id flips before the debounce fires', async () => {
    getSessionInfo.mockResolvedValue({ id: 'B', cwd: '/tmp', agentId: 'a' })
    const wrapper = mount(PaletteSessionsPreview, {
      props: { entry: { id: 'A', name: 'a' } }
    })

    vi.advanceTimersByTime(100)
    // vue-tsc setProps narrowing miss; same pattern as Frame.test.ts:153
    // @ts-expect-error vue-tsc setProps narrowing
    await wrapper.setProps({ entry: { id: 'B', name: 'b' } })
    vi.advanceTimersByTime(200)

    expect(getSessionInfo).toHaveBeenCalledTimes(1)
    expect(getSessionInfo).toHaveBeenCalledWith('B')

    await wrapper.unmount()
  })

  it('renders the daemon projection once the call resolves', async () => {
    getSessionInfo.mockResolvedValue({
      id: 'sess-1',
      title: 'feature work',
      cwd: '/home/u/dev',
      agentId: 'claude-code',
      profileId: 'strict'
    })
    const wrapper = mount(PaletteSessionsPreview, {
      props: { entry: { id: 'sess-1', name: 'feature work' } }
    })

    vi.advanceTimersByTime(200)
    await vi.runAllTimersAsync()
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('feature work')
    expect(wrapper.text()).toContain('claude-code')
    expect(wrapper.text()).toContain('strict')
  })

  it('renders an error indicator when the daemon rejects', async () => {
    getSessionInfo.mockRejectedValue(new Error('no such session'))
    const wrapper = mount(PaletteSessionsPreview, {
      props: { entry: { id: 'ghost', name: 'ghost' } }
    })

    vi.advanceTimersByTime(200)
    await vi.runAllTimersAsync()
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-testid="palette-sessions-preview-err"]').exists()).toBe(true)
  })
})
