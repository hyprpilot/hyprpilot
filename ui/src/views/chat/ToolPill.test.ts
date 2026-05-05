import { faTerminal } from '@fortawesome/free-solid-svg-icons'
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ToolPill from './ToolPill.vue'
import { PermissionUi, ToolKind, ToolState, type ToolCallView } from '@components'

function makeView(overrides: Partial<ToolCallView> = {}): ToolCallView {
  return {
    id: 'tc-1',
    kind: ToolKind.Read,
    name: 'Read',
    state: ToolState.Done,
    icon: faTerminal,
    permissionUi: PermissionUi.Row,
    title: 'read · src/App.vue',
    stats: [{ kind: 'duration', ms: 74 }],
    fields: [],
    ...overrides
  }
}

describe('ToolPill.vue', () => {
  it('renders icon + title + stat with the title as aria text', () => {
    const wrapper = mount(ToolPill, { props: { view: makeView() } })

    expect(wrapper.find('.tool-pill-icon-cell').attributes('aria-label')).toBe('read · src/App.vue')
    expect(wrapper.find('.tool-pill-icon').exists()).toBe(true)
    expect(wrapper.text()).toContain('read · src/App.vue')
    expect(wrapper.text()).toContain('74ms')
    expect(wrapper.attributes('data-state')).toBe('done')
    expect(wrapper.attributes('data-kind')).toBe(ToolKind.Read)
  })

  it('reflects state on the data attribute', () => {
    const wrapper = mount(ToolPill, { props: { view: makeView({ state: ToolState.Running }) } })

    expect(wrapper.attributes('data-state')).toBe('running')
  })
})
