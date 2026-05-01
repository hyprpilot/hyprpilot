import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatToolPill from './ToolPill.vue'
import { ToolKind, ToolState, type ToolChipItem } from '@components'

describe('ChatToolPill.vue', () => {
  it('renders icon + arg + stat with the label as aria text', () => {
    const item: ToolChipItem = { label: 'Read', arg: 'src/App.vue', stat: '74 ms', state: ToolState.Done, kind: ToolKind.Read }
    const wrapper = mount(ChatToolPill, { props: { item } })

    expect(wrapper.find('.tool-pill-name').attributes('aria-label')).toBe('Read')
    expect(wrapper.find('.tool-pill-icon').exists()).toBe(true)
    expect(wrapper.text()).toContain('src/App.vue')
    expect(wrapper.text()).toContain('74 ms')
    expect(wrapper.attributes('data-state')).toBe('done')
  })

  it('reflects state on the data attribute', () => {
    const item: ToolChipItem = { label: 'Read', state: ToolState.Running, kind: ToolKind.Read }
    const wrapper = mount(ChatToolPill, { props: { item } })

    expect(wrapper.attributes('data-state')).toBe('running')
  })
})
