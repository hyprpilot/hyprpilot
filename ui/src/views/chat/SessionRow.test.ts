import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import SessionRow from './SessionRow.vue'
import { Phase, type SessionRowData } from '@components'

const session: SessionRowData = {
  id: 's1',
  title: 'utils/md-printer',
  cwd: '~/dev/md-printer',
  adapter: 'claude-code',
  doing: 'planning tests',
  phase: Phase.Streaming
}

describe('SessionRow.vue', () => {
  it('renders all columns + the phase attribute', () => {
    const wrapper = mount(SessionRow, { props: { session } })

    expect(wrapper.attributes('data-phase')).toBe('streaming')
    expect(wrapper.text()).toContain('utils/md-printer')
    expect(wrapper.text()).toContain('~/dev/md-printer')
    expect(wrapper.text()).toContain('claude-code')
    expect(wrapper.text()).toContain('planning tests')
  })

  it('emits focus on click with the session id', async() => {
    const wrapper = mount(SessionRow, { props: { session } })

    await wrapper.trigger('click')

    expect(wrapper.emitted('focus')?.[0]).toEqual(['s1'])
  })
})
