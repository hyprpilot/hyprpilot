import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatToolRowBig from './ChatToolRowBig.vue'
import { ToolState, type ToolChipItem } from '../types'

describe('ChatToolRowBig.vue', () => {
  it('renders label / arg / stat / detail', () => {
    const item: ToolChipItem = { label: 'Bash', arg: 'pnpm test', stat: '1.4s', detail: 'exit 0', state: ToolState.Done, kind: 'bash' }
    const wrapper = mount(ChatToolRowBig, { props: { item } })

    expect(wrapper.text()).toContain('Bash')
    expect(wrapper.text()).toContain('pnpm test')
    expect(wrapper.text()).toContain('1.4s')
    expect(wrapper.text()).toContain('exit 0')
  })
})
