import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'

import { pushTerminalChunk, pushTerminalExit, resetTerminals } from '@composables'

import ChatTerminalCard from './TerminalCard.vue'

beforeEach(() => {
  resetTerminals('inst-A')
  resetTerminals('inst-B')
})

describe('ChatTerminalCard.vue', () => {
  it('renders the bound terminal entry while running', () => {
    pushTerminalChunk('inst-A', { terminalId: 't-1', data: 'running 12 specs...\n', command: 'pnpm test' })

    const wrapper = mount(ChatTerminalCard, {
      props: { terminalId: 't-1', instanceId: 'inst-A' }
    })

    expect(wrapper.text()).toContain('pnpm test')
    expect(wrapper.text()).toContain('running 12 specs')
    expect(wrapper.find('.terminal-card-cursor').exists()).toBe(true)
    expect(wrapper.find('.terminal-card-status-dot').attributes('data-state')).toBe('stream')
    expect(wrapper.find('button').text()).toBe('cancel')
  })

  it('shows exit + ok dot once the terminal completes cleanly', () => {
    pushTerminalChunk('inst-A', { terminalId: 't-2', data: 'done.', command: 'pnpm build' })
    pushTerminalExit('inst-A', { terminalId: 't-2', exitCode: 0 })

    const wrapper = mount(ChatTerminalCard, {
      props: { terminalId: 't-2', instanceId: 'inst-A' }
    })

    expect(wrapper.find('button').exists()).toBe(false)
    expect(wrapper.find('.terminal-card-status-dot').attributes('data-state')).toBe('ok')
    expect(wrapper.find('.terminal-card-exit').attributes('data-ok')).toBe('true')
    expect(wrapper.text()).toContain('exit 0')
    expect(wrapper.find('.terminal-card-cursor').exists()).toBe(false)
  })

  it('flips status dot to err on non-zero exit and surfaces signal text', () => {
    pushTerminalChunk('inst-A', { terminalId: 't-3', data: 'oops', command: 'sh -c "exit 137"' })
    pushTerminalExit('inst-A', { terminalId: 't-3', signal: 'SIGKILL' })

    const wrapper = mount(ChatTerminalCard, {
      props: { terminalId: 't-3', instanceId: 'inst-A' }
    })

    expect(wrapper.find('.terminal-card-status-dot').attributes('data-state')).toBe('err')
    expect(wrapper.text()).toContain('signal SIGKILL')
  })

  it('emits cancel on click', async () => {
    pushTerminalChunk('inst-A', { terminalId: 't-4', data: '', command: 'sleep 5' })

    const wrapper = mount(ChatTerminalCard, {
      props: { terminalId: 't-4', instanceId: 'inst-A' }
    })

    await wrapper.find('button').trigger('click')

    expect(wrapper.emitted('cancel')).toHaveLength(1)
  })

  it('renders the terminalId fallback when no command bound yet', () => {
    pushTerminalChunk('inst-A', { terminalId: 'no-cmd', data: 'foo' })

    const wrapper = mount(ChatTerminalCard, {
      props: { terminalId: 'no-cmd', instanceId: 'inst-A' }
    })

    expect(wrapper.text()).toContain('no-cmd')
  })
})
