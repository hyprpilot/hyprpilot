import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { Modifier, TauriCommand } from '@ipc'

import { __resetComposerForTests } from '@composables/use-composer'
import { __resetKeymapsForTests, useKeymaps } from '@composables/use-keymaps'

import ChatComposer from './ChatComposer.vue'
import { ComposerPillKind, type ComposerPill } from '../types'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@ipc', async () => ({
  ...(await vi.importActual<typeof import('@ipc')>('@ipc')),
  invoke: (...args: unknown[]) => invokeMock(...args)
}))

// Clipboard plugin lives behind the IPC bridge — mock the plugin
// surface so the Vitest jsdom environment doesn't try to talk to a
// real Tauri host.
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  readImage: vi.fn().mockRejectedValue(new Error('no clipboard host'))
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
  beforeEach(() => {
    __resetKeymapsForTests()
    __resetComposerForTests()
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === TauriCommand.GetKeymaps) {
        return Promise.resolve(DEFAULT_KEYMAPS)
      }
      return Promise.resolve(undefined)
    })
    // Seed the cache directly — loadKeymaps() goes through `invoke`
    // which the test harness fails to mock cleanly here. Direct write
    // bypasses the noise.
    useKeymaps().keymaps.value = DEFAULT_KEYMAPS as never
  })

  it('renders pills + removes them', async () => {
    const pills: ComposerPill[] = [
      { kind: ComposerPillKind.Attachment, id: 'a', label: 'file://src/App.vue', data: 'AA==', mimeType: 'image/png' },
      { kind: ComposerPillKind.Resource, id: 'b', label: 'skills/debug', data: 'debug', mimeType: 'skill' }
    ]
    const wrapper = mount(ChatComposer, { props: { pills } })

    expect(wrapper.findAll('.composer-pill')).toHaveLength(2)
    await wrapper.findAll('button[aria-label="remove"]')[0]!.trigger('click')
    expect(wrapper.emitted('removePill')?.[0]).toEqual(['a'])
  })

  it('exposes an addPill hook for external pill injection (Ctrl+P sink)', async () => {
    const wrapper = mount(ChatComposer)
    const vm = wrapper.vm as unknown as { addPill: (p: ComposerPill) => void }
    vm.addPill({
      kind: ComposerPillKind.Attachment,
      id: 'k-1',
      label: 'image/png · 4B',
      data: 'AAAA',
      mimeType: 'image/png'
    })
    await wrapper.vm.$nextTick()

    expect(wrapper.findAll('.composer-pill')).toHaveLength(1)
  })

  it('drag-drop of a non-image file is ignored (palette-only resources)', async () => {
    const wrapper = mount(ChatComposer)
    const form = wrapper.get('[data-testid="composer"]')

    const file = new File(['body'], 'notes.txt', { type: 'text/plain' })
    const dataTransfer = { files: [file], dropEffect: 'copy' } as unknown as DataTransfer

    const dropEvent = new Event('drop', { bubbles: true }) as unknown as DragEvent
    Object.defineProperty(dropEvent, 'dataTransfer', { value: dataTransfer })
    form.element.dispatchEvent(dropEvent)
    for (let i = 0; i < 8; i++) {
      await Promise.resolve()
    }
    await wrapper.vm.$nextTick()

    expect(wrapper.findAll('.composer-pill')).toHaveLength(0)
  })

  it('disables submit for empty or sending state', async () => {
    const wrapper = mount(ChatComposer, { props: { sending: true } })
    const submit = wrapper.get('[data-testid="composer-submit"]')

    expect(submit.attributes('disabled')).toBeDefined()
    expect(submit.attributes('aria-label')).toBe('sending')
  })

  it('emits submit with trimmed text payload', async () => {
    const wrapper = mount(ChatComposer)
    const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')

    await textarea.setValue('  hello  ')
    await wrapper.trigger('submit')
    for (let i = 0; i < 4; i++) {
      await Promise.resolve()
    }
    await wrapper.vm.$nextTick()

    const emitted = wrapper.emitted('submit')?.[0]
    expect(emitted).toBeDefined()
    expect((emitted as [{ text: string; attachments: unknown[] }])[0]).toMatchObject({
      text: 'hello',
      attachments: []
    })
  })

  it('Enter submits; Shift+Enter does not', async () => {
    const wrapper = mount(ChatComposer, { attachTo: document.body })
    const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')
    await textarea.setValue('hi')
    textarea.element.focus()

    await textarea.trigger('keydown', { key: 'Enter', shiftKey: true })
    expect(wrapper.emitted('submit')).toBeUndefined()

    await textarea.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('submit')?.[0]).toBeDefined()
  })
})
