import { faTerminal } from '@fortawesome/free-solid-svg-icons'
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ToolChips from './ToolChips.vue'
import { PermissionUi, ToolKind, ToolState, type ToolCallView } from '@components'

function makeView(id: string): ToolCallView {
  return {
    id,
    kind: ToolKind.Read,
    name: 'Read',
    state: ToolState.Done,
    icon: faTerminal,
    permissionUi: PermissionUi.Row,
    title: `read · ${id}`,
    stats: [],
    fields: [{ label: 'path', value: id }]
  }
}

describe('ToolChips.vue', () => {
  it('keeps the turn-level tools card expanded by default', () => {
    const wrapper = mount(ToolChips, {
      props: {
        views: [makeView('one')]
      },
      global: {
        stubs: {
          ToolPill: {
            props: ['view'],
            template: '<div class="stub-tool-pill" :data-id="view.id" />'
          }
        }
      }
    })

    expect(wrapper.attributes('data-expanded')).toBe('true')
    expect(wrapper.find('.stub-tool-pill').exists()).toBe(true)
  })
})
