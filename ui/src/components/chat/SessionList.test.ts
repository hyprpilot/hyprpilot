import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import SessionList from '@components/chat/SessionList.vue'
import type { SessionSummary } from '@ipc'

const items: SessionSummary[] = [
  { sessionId: 'sess-1', cwd: '/tmp/a', title: 'first session', updatedAt: '2026-04-22T12:00:00Z' },
  { sessionId: 'sess-2', cwd: '/tmp/b', title: 'second session' }
]

describe('SessionList.vue', () => {
  it('renders the empty state when there are no sessions', () => {
    const wrapper = mount(SessionList, { props: { sessions: [] } })
    expect(wrapper.find('[data-testid="session-list-empty"]').exists()).toBe(true)
    expect(wrapper.get('[data-testid="session-list-empty"]').text()).toContain('no saved sessions yet')
  })

  it('renders one entry per session with title + optional timestamp', () => {
    const wrapper = mount(SessionList, { props: { sessions: items } })

    expect(wrapper.get('[data-testid="session-list-item-sess-1"]').text()).toContain('first session')
    expect(wrapper.get('[data-testid="session-list-item-sess-1"]').text()).toContain('2026-04-22T12:00:00Z')
    expect(wrapper.get('[data-testid="session-list-item-sess-2"]').text()).toContain('second session')
  })

  it('emits load with the clicked sessionId', async () => {
    const wrapper = mount(SessionList, { props: { sessions: items } })
    await wrapper.get('[data-testid="session-list-item-sess-2"]').trigger('click')

    const emitted = wrapper.emitted('load')
    expect(emitted).toBeDefined()
    expect(emitted![0]).toEqual(['sess-2'])
  })

  it('does not emit load when the active session is clicked', async () => {
    const wrapper = mount(SessionList, { props: { sessions: items, activeSessionId: 'sess-1' } })
    await wrapper.get('[data-testid="session-list-item-sess-1"]').trigger('click')
    expect(wrapper.emitted('load')).toBeUndefined()
  })
})
