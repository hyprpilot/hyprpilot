import { faTerminal } from '@fortawesome/free-solid-svg-icons'
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ToolDetails from './ToolDetails.vue'
import { PermissionUi, PillKind, ToolKind, ToolState, type ToolCallView } from '@components'

describe('ToolDetails.vue', () => {
  it('renders title and duration stat', () => {
    const view: ToolCallView = {
      id: 'tc-1',
      kind: ToolKind.Execute,
      name: 'Bash',
      state: ToolState.Done,
      icon: faTerminal,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title: 'bash · pnpm test',
      stats: [{ kind: 'duration', ms: 1400 }],
      fields: []
    }
    const wrapper = mount(ToolDetails, { props: { view } })

    expect(wrapper.text()).toContain('bash · pnpm test')
    expect(wrapper.text()).toContain('1s')
  })
})
