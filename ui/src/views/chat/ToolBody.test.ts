import { faTerminal } from '@fortawesome/free-solid-svg-icons'
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ToolBody from './ToolBody.vue'
import { PermissionUi, ToolKind, ToolState, type ToolCallView } from '@components'

function makeView(overrides: Partial<ToolCallView> = {}): ToolCallView {
  return {
    id: 'tc-1',
    kind: ToolKind.Execute,
    name: 'Bash',
    state: ToolState.Done,
    icon: faTerminal,
    permissionUi: PermissionUi.Row,
    title: 'bash · ls',
    stats: [],
    fields: [],
    ...overrides
  }
}

describe('ToolBody.vue', () => {
  /**
   * Output expansion mirrors the parent pill's "live show, finalized
   * hide" policy: while the tool is running / awaiting permission
   * the captain wants to see the streaming stdout in real time;
   * once the call finalizes (Done / Failed) output collapses so a
   * long log doesn't dominate the chat — captain re-expands manually.
   */
  it('expands output on Running state to show streaming stdout', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          state: ToolState.Running,
          description: '```bash\nls /tmp\n```',
          output: 'lots\nof\nstdout\nlines\n...'
        })
      }
    })

    const section = wrapper.find('.tool-body-output')

    expect(section.exists()).toBe(true)
    expect(section.attributes('data-expanded')).toBe('true')
    expect(wrapper.find('.tool-body-output-body').exists()).toBe(true)
  })

  it('collapses output on Done state to free chat real estate', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          state: ToolState.Done,
          description: '```bash\nls /tmp\n```',
          output: 'lots\nof\nstdout\nlines\n...'
        })
      }
    })

    const section = wrapper.find('.tool-body-output')

    expect(section.attributes('data-expanded')).toBe('false')
    expect(wrapper.find('.tool-body-output-body').exists()).toBe(false)
  })

  it('collapses output on Failed state', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          state: ToolState.Failed,
          output: 'error: out of memory'
        })
      }
    })

    expect(wrapper.find('.tool-body-output').attributes('data-expanded')).toBe('false')
  })

  it('renders description before fields before output', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          description: 'desc-text',
          fields: [{ label: 'path', value: '/x' }],
          output: 'out'
        })
      }
    })

    const order = wrapper.element.querySelectorAll('.tool-body-description, .tool-body-fields, .tool-body-output')

    expect(order.length).toBe(3)
    expect((order[0] as HTMLElement).className).toContain('tool-body-description')
    expect((order[1] as HTMLElement).className).toContain('tool-body-fields')
    expect((order[2] as HTMLElement).className).toContain('tool-body-output')
  })

  it('renders fields with each label and value', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          fields: [
            { label: 'path', value: '/etc/hosts' },
            { label: 'pattern', value: '127.*' }
          ]
        })
      }
    })

    const labels = wrapper.findAll('.tool-body-label').map((n) => n.text())
    const codes = wrapper.findAll('.tool-body-code').map((n) => n.text())

    expect(labels).toEqual(['path', 'pattern'])
    expect(codes).toEqual(['/etc/hosts', '127.*'])
  })

  it('renders nothing visible when description / fields / output are all empty', () => {
    const wrapper = mount(ToolBody, { props: { view: makeView() } })

    expect(wrapper.find('.tool-body').exists()).toBe(false)
  })
})
