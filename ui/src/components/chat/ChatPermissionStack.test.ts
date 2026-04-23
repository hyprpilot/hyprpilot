import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatPermissionStack from './ChatPermissionStack.vue'
import type { PermissionPrompt } from '../types'

const prompts: PermissionPrompt[] = [
  { id: 'p1', tool: 'Write', kind: 'edit', args: 'src/App.vue' },
  { id: 'p2', tool: 'Bash', kind: 'execute', args: 'pnpm test', queued: true },
  { id: 'p3', tool: 'Write', kind: 'edit', args: 'README.md', queued: true }
]

describe('ChatPermissionStack.vue', () => {
  it('does not render when there are no prompts', () => {
    const wrapper = mount(ChatPermissionStack, { props: { prompts: [] } })

    expect(wrapper.find('[data-testid="permission-stack"]').exists()).toBe(false)
  })

  it('shows the pending count + renders every prompt', () => {
    const wrapper = mount(ChatPermissionStack, { props: { prompts } })

    expect(wrapper.text()).toContain('3 pending')
    expect(wrapper.findAll('li')).toHaveLength(3)
  })

  it('only shows allow/deny on the oldest non-queued row', () => {
    const wrapper = mount(ChatPermissionStack, { props: { prompts } })

    const active = wrapper.findAll('li').filter((row) => row.attributes('data-active') === 'true')
    expect(active).toHaveLength(1)
    expect(active[0]!.text()).toContain('allow')
    expect(active[0]!.text()).toContain('deny')
    expect(active[0]!.text()).toContain('src/App.vue')
  })

  it('emits allow / deny with the prompt id', async () => {
    const wrapper = mount(ChatPermissionStack, { props: { prompts } })

    const buttons = wrapper.findAll('button')
    await buttons[0]!.trigger('click')
    await buttons[1]!.trigger('click')

    expect(wrapper.emitted('allow')?.[0]).toEqual(['p1'])
    expect(wrapper.emitted('deny')?.[0]).toEqual(['p1'])
  })
})
