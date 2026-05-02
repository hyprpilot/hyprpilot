import { faTerminal } from '@fortawesome/free-solid-svg-icons'
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ToolPill from './ToolPill.vue'
import { PermissionUi, PillKind, ToolState, ToolType, type ToolCallView } from '@components'

function makeView(overrides: Partial<ToolCallView> = {}): ToolCallView {
  return {
    id: 'tc-1',
    type: ToolType.Read,
    name: 'Read',
    state: ToolState.Done,
    icon: faTerminal,
    pill: PillKind.Default,
    permissionUi: PermissionUi.Row,
    title: 'read · src/App.vue',
    stat: '74 ms',
    ...overrides
  }
}

describe('ToolPill.vue', () => {
  it('renders icon + title + stat with the title as aria text', () => {
    const wrapper = mount(ToolPill, { props: { view: makeView() } })

    expect(wrapper.find('.tool-pill-icon-cell').attributes('aria-label')).toBe('read · src/App.vue')
    expect(wrapper.find('.tool-pill-icon').exists()).toBe(true)
    expect(wrapper.text()).toContain('read · src/App.vue')
    expect(wrapper.text()).toContain('74 ms')
    expect(wrapper.attributes('data-state')).toBe('done')
    expect(wrapper.attributes('data-type')).toBe(ToolType.Read)
  })

  it('reflects state on the data attribute', () => {
    const wrapper = mount(ToolPill, { props: { view: makeView({ state: ToolState.Running }) } })

    expect(wrapper.attributes('data-state')).toBe('running')
  })
})
