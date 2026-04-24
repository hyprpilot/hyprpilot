import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatToolPillSmall from './ChatToolPillSmall.vue'
import { ToolKind, ToolState, type ToolChipItem } from '../types'

describe('ChatToolPillSmall.vue', () => {
  it('renders label / arg / stat', () => {
    const item: ToolChipItem = { label: 'R', arg: 'src/App.vue', stat: '74 ms', state: ToolState.Done, kind: ToolKind.Read }
    const wrapper = mount(ChatToolPillSmall, { props: { item } })

    expect(wrapper.text()).toContain('R')
    expect(wrapper.text()).toContain('src/App.vue')
    expect(wrapper.text()).toContain('74 ms')
    expect(wrapper.attributes('data-state')).toBe('done')
  })

  it('reflects state on the data attribute', () => {
    const item: ToolChipItem = { label: 'R', state: ToolState.Running, kind: ToolKind.Read }
    const wrapper = mount(ChatToolPillSmall, { props: { item } })

    expect(wrapper.attributes('data-state')).toBe('running')
  })
})
