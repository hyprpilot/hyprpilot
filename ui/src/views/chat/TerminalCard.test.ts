import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'

import ChatTerminalCard from './TerminalCard.vue'
import { pushTerminalChunk, pushTerminalExit, resetTerminals } from '@composables'

// Stub XtermView — TerminalCard tests verify wiring (props passed
// through to the xterm host), not xterm's actual render output. The
// real component is integration-tested by manually viewing a live
// daemon; here we just capture the props it received. `vi.mock` is
// auto-hoisted by vitest above the import block, so it intercepts
// ChatTerminalCard's transitive `@components` import.
vi.mock('@components', async() => {
  const actual = await vi.importActual<Record<string, unknown>>('@components')

  return {
    ...actual,
    XtermView: defineComponent({
      name: 'XtermViewStub',
      props: {
        text: { type: String, default: '' },
        rows: { type: Number, default: 16 },
        running: { type: Boolean, default: false }
      },
      render() {
        return h('div', {
          class: 'xterm-stub',
          'data-text': this.text,
          'data-running': this.running ? 'true' : 'false',
          'data-rows': String(this.rows)
        })
      }
    })
  }
})

beforeEach(() => {
  resetTerminals('inst-A')
  resetTerminals('inst-B')
})

describe('ChatTerminalCard.vue', () => {
  it('renders the bound terminal entry while running', () => {
    pushTerminalChunk('inst-A', {
      terminalId: 't-1',
      data: 'running 12 specs...\n',
      command: 'pnpm test'
    })

    const wrapper = mount(ChatTerminalCard, {
      props: { terminalId: 't-1', instanceId: 'inst-A' }
    })

    expect(wrapper.text()).toContain('pnpm test')
    const xterm = wrapper.find('.xterm-stub')

    expect(xterm.exists()).toBe(true)
    expect(xterm.attributes('data-text')).toBe('running 12 specs...\n')
    expect(xterm.attributes('data-running')).toBe('true')
    expect(wrapper.find('.terminal-card-status-dot').attributes('data-state')).toBe('stream')
    expect(wrapper.find('button').text()).toBe('cancel')
  })

  it('shows exit + ok dot once the terminal completes cleanly', () => {
    pushTerminalChunk('inst-A', {
      terminalId: 't-2',
      data: 'done.',
      command: 'pnpm build'
    })
    pushTerminalExit('inst-A', { terminalId: 't-2', exitCode: 0 })

    const wrapper = mount(ChatTerminalCard, {
      props: { terminalId: 't-2', instanceId: 'inst-A' }
    })

    expect(wrapper.find('button').exists()).toBe(false)
    expect(wrapper.find('.terminal-card-status-dot').attributes('data-state')).toBe('ok')
    expect(wrapper.find('.terminal-card-exit').attributes('data-ok')).toBe('true')
    expect(wrapper.text()).toContain('exit 0')
    expect(wrapper.find('.xterm-stub').attributes('data-running')).toBe('false')
  })

  it('flips status dot to err on non-zero exit and surfaces signal text', () => {
    pushTerminalChunk('inst-A', {
      terminalId: 't-3',
      data: 'oops',
      command: 'sh -c "exit 137"'
    })
    pushTerminalExit('inst-A', { terminalId: 't-3', signal: 'SIGKILL' })

    const wrapper = mount(ChatTerminalCard, {
      props: { terminalId: 't-3', instanceId: 'inst-A' }
    })

    expect(wrapper.find('.terminal-card-status-dot').attributes('data-state')).toBe('err')
    expect(wrapper.text()).toContain('signal SIGKILL')
  })

  it('emits cancel on click', async() => {
    pushTerminalChunk('inst-A', {
      terminalId: 't-4',
      data: '',
      command: 'sleep 5'
    })

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

  it('passes ANSI-bearing output through to xterm verbatim (no stripping)', () => {
    // Real terminal output frequently carries ANSI color codes — npm
    // progress, cargo build, pytest. The previous TerminalCard ran a
    // tiny `stripAnsi` shim before rendering, which lost every escape
    // and showed garbled output. xterm.js renders the escapes
    // properly; this test pins that the card forwards `output`
    // unchanged so the renderer can do its job.
    const ansi = '\u001b[31mERROR\u001b[0m something went wrong'

    pushTerminalChunk('inst-A', {
      terminalId: 't-ansi',
      data: ansi,
      command: 'cargo build'
    })

    const wrapper = mount(ChatTerminalCard, {
      props: { terminalId: 't-ansi', instanceId: 'inst-A' }
    })

    expect(wrapper.find('.xterm-stub').attributes('data-text')).toBe(ansi)
  })
})
