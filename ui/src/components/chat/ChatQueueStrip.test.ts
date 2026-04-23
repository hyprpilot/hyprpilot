import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatQueueStrip from './ChatQueueStrip.vue'
import type { QueuedMessage } from '../types'

const messages: QueuedMessage[] = [
  { id: 'q1', text: 'fix that' },
  { id: 'q2', text: 'also run tests' }
]

describe('ChatQueueStrip.vue', () => {
  it('does not render when empty', () => {
    const wrapper = mount(ChatQueueStrip, { props: { messages: [] } })

    expect(wrapper.find('[data-testid="queue-strip"]').exists()).toBe(false)
  })

  it('renders a row per message + header count', () => {
    const wrapper = mount(ChatQueueStrip, { props: { messages } })

    expect(wrapper.text()).toContain('QUEUED · 2')
    expect(wrapper.findAll('.queue-strip-row')).toHaveLength(2)
    expect(wrapper.text()).toContain('fix that')
    expect(wrapper.text()).toContain('also run tests')
  })

  it('emits row-scoped and global action events', async () => {
    const wrapper = mount(ChatQueueStrip, { props: { messages } })

    await wrapper.get('.queue-strip-drop-all').trigger('click')
    const rowActions = wrapper.findAll('.queue-strip-row button')
    // [edit q1, send q1, drop q1, edit q2, ...]
    await rowActions[0]!.trigger('click') // edit q1
    await rowActions[1]!.trigger('click') // send q1
    await rowActions[2]!.trigger('click') // drop q1

    expect(wrapper.emitted('dropAll')).toHaveLength(1)
    expect(wrapper.emitted('edit')?.[0]).toEqual(['q1'])
    expect(wrapper.emitted('send')?.[0]).toEqual(['q1'])
    expect(wrapper.emitted('drop')?.[0]).toEqual(['q1'])
  })
})
