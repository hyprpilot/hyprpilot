import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatTerminalCard from './ChatTerminalCard.vue'

describe('ChatTerminalCard.vue', () => {
  it('renders command + stdout + cursor while running', () => {
    const wrapper = mount(ChatTerminalCard, {
      props: { command: 'pnpm test', stdout: 'running 12 specs...\n', running: true }
    })

    expect(wrapper.text()).toContain('pnpm test')
    expect(wrapper.text()).toContain('running 12 specs')
    expect(wrapper.find('.terminal-card-cursor').exists()).toBe(true)
    expect(wrapper.find('button').text()).toBe('cancel')
  })

  it('hides the cancel button and shows exit code when finished', () => {
    const wrapper = mount(ChatTerminalCard, {
      props: { command: 'pnpm test', stdout: 'done.', running: false, exitCode: 0 }
    })

    expect(wrapper.find('button').exists()).toBe(false)
    expect(wrapper.find('.terminal-card-exit').attributes('data-ok')).toBe('true')
    expect(wrapper.text()).toContain('exit 0')
    expect(wrapper.find('.terminal-card-cursor').exists()).toBe(false)
  })

  it('emits cancel on click', async () => {
    const wrapper = mount(ChatTerminalCard, { props: { command: 'sleep 5', stdout: '' } })

    await wrapper.find('button').trigger('click')

    expect(wrapper.emitted('cancel')).toHaveLength(1)
  })
})
