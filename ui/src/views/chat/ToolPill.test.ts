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

  /**
   * Auto-expand policy: live calls show their guts, finalized calls
   * collapse. Captain wants to watch streaming output as it lands
   * (Running / Awaiting) and reclaim chat space once the call
   * finishes — the status indicator (border tone + stat pills)
   * communicates the outcome at-a-glance, expanding for details
   * is a deliberate drill-in.
   */
  it('expands on Running state', () => {
    const wrapper = mount(ToolPill, { props: { view: makeView({ state: ToolState.Running }) } })

    expect(wrapper.attributes('data-expanded')).toBe('true')
  })

  it('expands on Awaiting state', () => {
    const wrapper = mount(ToolPill, { props: { view: makeView({ state: ToolState.Awaiting }) } })

    expect(wrapper.attributes('data-expanded')).toBe('true')
  })

  it('collapses on Done state regardless of description / fields content', () => {
    const wrapper = mount(ToolPill, {
      props: {
        view: makeView({
          state: ToolState.Done,
          description: '```bash\nls /tmp\n```',
          fields: [{ label: 'path', value: '/etc/hosts' }]
        })
      }
    })

    expect(wrapper.attributes('data-expanded')).toBe('false')
  })

  it('collapses on Failed state', () => {
    const wrapper = mount(ToolPill, { props: { view: makeView({ state: ToolState.Failed }) } })

    expect(wrapper.attributes('data-expanded')).toBe('false')
  })
})
