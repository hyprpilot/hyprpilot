import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatToolPillSmall from './ChatToolPillSmall.vue'
import { ToolState, type ToolChipItem } from '../types'

describe('ChatToolPillSmall.vue', () => {
  it('renders label / arg / stat', () => {
    const item: ToolChipItem = { label: 'Read', arg: 'src/App.vue', stat: '74 ms', state: ToolState.Done, kind: 'read' }
    const wrapper = mount(ChatToolPillSmall, { props: { item } })

    expect(wrapper.text()).toContain('Read')
    expect(wrapper.text()).toContain('src/App.vue')
    expect(wrapper.text()).toContain('74 ms')
    expect(wrapper.attributes('data-state')).toBe('done')
  })

  it('reflects state on the data attribute', () => {
    const item: ToolChipItem = { label: 'Read', state: ToolState.Running, kind: 'read' }
    const wrapper = mount(ChatToolPillSmall, { props: { item } })

    expect(wrapper.attributes('data-state')).toBe('running')
  })
})
