import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatLiveSessionRow from './ChatLiveSessionRow.vue'
import { Phase, type LiveSession } from '../types'

const session: LiveSession = {
  id: 's1',
  title: 'utils/md-printer',
  cwd: '~/dev/md-printer',
  adapter: 'claude-code',
  doing: 'planning tests',
  phase: Phase.Streaming
}

describe('ChatLiveSessionRow.vue', () => {
  it('renders all columns + the phase attribute', () => {
    const wrapper = mount(ChatLiveSessionRow, { props: { session } })

    expect(wrapper.attributes('data-phase')).toBe('streaming')
    expect(wrapper.text()).toContain('utils/md-printer')
    expect(wrapper.text()).toContain('~/dev/md-printer')
    expect(wrapper.text()).toContain('claude-code')
    expect(wrapper.text()).toContain('planning tests')
  })

  it('emits focus on click with the session id', async () => {
    const wrapper = mount(ChatLiveSessionRow, { props: { session } })

    await wrapper.trigger('click')

    expect(wrapper.emitted('focus')?.[0]).toEqual(['s1'])
  })
})
