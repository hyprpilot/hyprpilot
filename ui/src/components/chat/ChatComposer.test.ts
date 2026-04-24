import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { Modifier, TauriCommand } from '@ipc'

import { __resetKeymapsForTests, loadKeymaps } from '@composables/useKeymaps'

import ChatComposer from './ChatComposer.vue'
import type { ComposerPill } from '../types'

const invokeMock = vi.fn()

vi.mock('@ipc', async () => ({
  ...(await vi.importActual<typeof import('@ipc')>('@ipc')),
  invoke: (...args: unknown[]) => invokeMock(...args)
}))

const DEFAULT_KEYMAPS = {
  chat: {
    submit: { modifiers: [], key: 'enter' },
    newline: { modifiers: [Modifier.Shift], key: 'enter' }
  },
  approvals: {
    allow: { modifiers: [], key: 'a' },
    deny: { modifiers: [], key: 'd' }
  },
  composer: {
    paste_image: { modifiers: [Modifier.Ctrl], key: 'p' },
    tab_completion: { modifiers: [], key: 'tab' },
    shift_tab: { modifiers: [Modifier.Shift], key: 'tab' },
    history_up: { modifiers: [Modifier.Ctrl], key: 'arrowup' },
    history_down: { modifiers: [Modifier.Ctrl], key: 'arrowdown' }
  },
  palette: {
    open: { modifiers: [Modifier.Ctrl], key: 'k' },
    close: { modifiers: [], key: 'escape' },
    models: { focus: { modifiers: [Modifier.Ctrl], key: 'm' } },
    sessions: { focus: { modifiers: [Modifier.Ctrl], key: 's' } }
  },
  transcript: {}
}

describe('ChatComposer.vue', () => {
  beforeEach(async () => {
    __resetKeymapsForTests()
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === TauriCommand.GetKeymaps) {
        return Promise.resolve(DEFAULT_KEYMAPS)
      }
      return Promise.resolve(undefined)
    })
    await loadKeymaps()
  })

  it('renders pills + removes them', async () => {
    const pills: ComposerPill[] = [
      { id: 'a', label: 'file://src/App.vue', kind: 'attachment' },
      { id: 'b', label: 'skills/debug', kind: 'skill' }
    ]
    const wrapper = mount(ChatComposer, { props: { pills } })

    expect(wrapper.findAll('.composer-pill')).toHaveLength(2)
    await wrapper.findAll('button[aria-label="remove"]')[0]!.trigger('click')
    expect(wrapper.emitted('removePill')?.[0]).toEqual(['a'])
  })

  it('disables submit for empty or sending state', async () => {
    const wrapper = mount(ChatComposer, { props: { sending: true } })
    const submit = wrapper.get('[data-testid="composer-submit"]')

    expect(submit.attributes('disabled')).toBeDefined()
    expect(submit.attributes('aria-label')).toBe('sending')
  })

  it('emits submit with trimmed text', async () => {
    const wrapper = mount(ChatComposer)
    const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')

    await textarea.setValue('  hello  ')
    await wrapper.trigger('submit')

    expect(wrapper.emitted('submit')?.[0]).toEqual(['hello'])
  })

  it('enter submits, shift+enter does not', async () => {
    const wrapper = mount(ChatComposer, { attachTo: document.body })
    const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')
    await textarea.setValue('hi')
    textarea.element.focus()

    await textarea.trigger('keydown', { key: 'Enter', shiftKey: true })
    expect(wrapper.emitted('submit')).toBeUndefined()

    await textarea.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('submit')?.[0]).toEqual(['hi'])
    wrapper.unmount()
  })
})
