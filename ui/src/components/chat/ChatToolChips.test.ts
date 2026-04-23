import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatToolChips from './ChatToolChips.vue'
import { ToolState, type ToolChipItem } from '../types'

describe('ChatToolChips.vue', () => {
  it('groups consecutive small tools into a flex-wrap row and breaks around big tools', () => {
    const items: ToolChipItem[] = [
      { label: 'Read', state: ToolState.Done, kind: 'read' },
      { label: 'Glob', state: ToolState.Done, kind: 'search' },
      { label: 'Bash', arg: 'pnpm test', state: ToolState.Running, kind: 'bash' },
      { label: 'Read', state: ToolState.Done, kind: 'read' }
    ]
    const wrapper = mount(ChatToolChips, { props: { items } })

    const rows = wrapper.findAll('.tool-chips-small-row')
    expect(rows).toHaveLength(2)
    // First row has 2 small tools, second row has 1.
    expect(rows[0]!.findAll('.tool-pill-small')).toHaveLength(2)
    expect(rows[1]!.findAll('.tool-pill-small')).toHaveLength(1)
    // Exactly 1 big row between them.
    expect(wrapper.findAll('.tool-row-big')).toHaveLength(1)
    expect(wrapper.find('.tool-row-big').text()).toContain('Bash')
  })

  it('packs 4 small chips into a single flex-wrap row', () => {
    const items: ToolChipItem[] = [
      { label: 'Read', state: ToolState.Done, kind: 'read' },
      { label: 'Grep', state: ToolState.Done, kind: 'search' },
      { label: 'Glob', state: ToolState.Done, kind: 'search' },
      { label: 'List', state: ToolState.Done, kind: 'read' }
    ]
    const wrapper = mount(ChatToolChips, { props: { items } })

    const rows = wrapper.findAll('.tool-chips-small-row')
    expect(rows).toHaveLength(1)
    expect(rows[0]!.findAll('.tool-pill-small')).toHaveLength(4)
  })

  it('handles an empty list', () => {
    const wrapper = mount(ChatToolChips, { props: { items: [] } })

    expect(wrapper.find('[data-testid="tool-chips"]').exists()).toBe(true)
    expect(wrapper.findAll('.tool-chips-small-row')).toHaveLength(0)
  })
})
