import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatToolChips from './ChatToolChips.vue'
import { ToolKind, ToolState, type ToolChipItem } from '../types'

describe('ChatToolChips.vue', () => {
  it('groups consecutive small tools into a flex-wrap row and breaks around big-kind tools', () => {
    const items: ToolChipItem[] = [
      { label: 'R', state: ToolState.Done, kind: ToolKind.Read },
      { label: '/', state: ToolState.Done, kind: ToolKind.Search },
      { label: '$', arg: 'pnpm test', state: ToolState.Running, kind: ToolKind.Bash },
      { label: 'R', state: ToolState.Done, kind: ToolKind.Read }
    ]
    const wrapper = mount(ChatToolChips, { props: { items } })

    const rows = wrapper.findAll('.tool-chips-small-row')
    expect(rows).toHaveLength(2)
    // First row has 2 small tools, second row has 1.
    expect(rows[0]!.findAll('.tool-pill-small')).toHaveLength(2)
    expect(rows[1]!.findAll('.tool-pill-small')).toHaveLength(1)
    // Exactly 1 big row between them — the Bash-kind item.
    expect(wrapper.findAll('.tool-row-big')).toHaveLength(1)
    expect(wrapper.find('.tool-row-big').text()).toContain('pnpm test')
  })

  it('promotes every BIG_KINDS variant to a big row', () => {
    const items: ToolChipItem[] = [
      { label: '$', state: ToolState.Done, kind: ToolKind.Bash },
      { label: '⇲', state: ToolState.Done, kind: ToolKind.Write },
      { label: '›_', state: ToolState.Done, kind: ToolKind.Terminal }
    ]
    const wrapper = mount(ChatToolChips, { props: { items } })

    expect(wrapper.findAll('.tool-row-big')).toHaveLength(3)
    expect(wrapper.findAll('.tool-chips-small-row')).toHaveLength(0)
  })

  it('packs 4 small chips into a single flex-wrap row', () => {
    const items: ToolChipItem[] = [
      { label: 'R', state: ToolState.Done, kind: ToolKind.Read },
      { label: '/', state: ToolState.Done, kind: ToolKind.Search },
      { label: '△', state: ToolState.Done, kind: ToolKind.Search },
      { label: 'R', state: ToolState.Done, kind: ToolKind.Read }
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
