import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import ChatComposer from './Composer.vue'
import { ComposerPillKind, type ComposerPill } from '@components'
import { __resetComposerForTests, __resetKeymapsForTests, __resetUseCompletionForTests, useKeymaps } from '@composables'
import { CompletionKind, Modifier, TauriCommand } from '@ipc'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

const EMPTY_COMPLETION_RESPONSE = {
  requestId: 'completion-empty',
  sourceId: null,
  replacementRange: null,
  items: []
}

vi.mock('@ipc', async() => ({
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
    newline: { modifiers: [Modifier.Shift], key: 'enter' },
    cancel_turn: { modifiers: [Modifier.Ctrl], key: 'c' },
    focus_input: { modifiers: [Modifier.Ctrl], key: 'f' }
  },
  approvals: {
    allow: { modifiers: [], key: 'a' },
    deny: { modifiers: [], key: 'd' }
  },
  composer: {
    paste: { modifiers: [Modifier.Ctrl], key: 'p' },
    tab_completion: { modifiers: [], key: 'tab' },
    shift_tab: { modifiers: [Modifier.Shift], key: 'tab' },
    completion: { modifiers: [Modifier.Ctrl], key: 'space' },
    history_up: { modifiers: [Modifier.Ctrl], key: 'arrowup' },
    history_down: { modifiers: [Modifier.Ctrl], key: 'arrowdown' }
  },
  palette: {
    open: { modifiers: [Modifier.Ctrl], key: 'k' },
    close: { modifiers: [], key: 'escape' },
    instances: { focus: { modifiers: [Modifier.Ctrl], key: 'i' } }
  },
  transcript: {}
}

describe('ChatComposer.vue', () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn()
    __resetKeymapsForTests()
    __resetComposerForTests()
    __resetUseCompletionForTests()
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === TauriCommand.GetKeymaps) {
        return Promise.resolve(DEFAULT_KEYMAPS)
      }

      if (cmd === TauriCommand.CompletionQuery) {
        return Promise.resolve(EMPTY_COMPLETION_RESPONSE)
      }

      return Promise.resolve(undefined)
    })
    // Seed the cache directly — loadKeymaps() goes through `invoke`
    // which the test harness fails to mock cleanly here. Direct write
    // bypasses the noise.
    useKeymaps().keymaps.value = DEFAULT_KEYMAPS as never
  })

  afterEach(() => {
    __resetUseCompletionForTests()
    document.body.innerHTML = ''
  })

  it('renders pills + removes them', async() => {
    const pills: ComposerPill[] = [
      {
        kind: ComposerPillKind.Attachment,
        id: 'a',
        label: 'file://src/App.vue',
        data: 'AA==',
        mimeType: 'image/png'
      },
      {
        kind: ComposerPillKind.Resource,
        id: 'b',
        label: 'skills/debug',
        data: 'debug',
        mimeType: 'skill'
      }
    ]
    const wrapper = mount(ChatComposer, { props: { pills } })

    expect(wrapper.findAll('.composer-pill')).toHaveLength(2)
    await wrapper.findAll('button[aria-label="remove"]')[0]!.trigger('click')
    expect(wrapper.emitted('removePill')?.[0]).toEqual(['a'])
  })

  it('exposes an addPill hook for external pill injection (Ctrl+P sink)', async() => {
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

  it('drag-drop of a non-image file is ignored (palette-only resources)', async() => {
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

  it('disables submit for empty or sending state', async() => {
    const wrapper = mount(ChatComposer, { props: { sending: true } })
    const submit = wrapper.get('[data-testid="composer-submit"]')

    expect(submit.attributes('disabled')).toBeDefined()
    expect(submit.attributes('aria-label')).toBe('sending')
  })

  it('blocks submit on a fully empty buffer even when attachments are pending', async() => {
    // Pin the rule: SOMETHING must be typed for a submit to fire.
    // Attachments / pills alone don't qualify — an empty-buffer
    // prompt lands on the daemon as a turn with no user content,
    // corrupting the boundary on subsequent prompts.
    //
    // Whitespace counts as text. Captains who deliberately type
    // spaces (e.g. as a leading-newline workaround on a coarse
    // keyboard) can send; we gate on raw `text.length === 0`, not
    // a trim. `resolvedSubmit` still trims for the wire payload.
    const wrapper = mount(ChatComposer)

    const pill: ComposerPill = {
      id: 'pill-1',
      label: 'image.png',
      kind: ComposerPillKind.Attachment,
      data: 'AAAA',
      mimeType: 'image/png'
    }

    ;(wrapper.vm as unknown as { addPill: (p: ComposerPill) => void }).addPill(pill)
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('.composer-pill')).toHaveLength(1)

    // Submit button is disabled — empty buffer + a pill is NOT enough.
    const submit = wrapper.get('[data-testid="composer-submit"]')

    expect(submit.attributes('disabled')).toBeDefined()
    expect(submit.attributes('data-empty')).toBe('true')

    // Form-submit attempt on empty buffer must NOT emit.
    await wrapper.trigger('submit')

    for (let i = 0; i < 4; i++) {
      await Promise.resolve()
    }
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('submit')).toBeUndefined()

    // Whitespace IS valid text — the button enables once anything
    // is typed. The wire payload is whatever `resolvedSubmit`
    // produces (trimmed for the wire), but the gate is raw length.
    const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')

    await textarea.setValue('   ')
    expect(submit.attributes('disabled')).toBeUndefined()
    expect(submit.attributes('data-empty')).toBe('false')
  })

  it('emits submit with trimmed text payload', async() => {
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

  it('keeps the first Ctrl+Space selection when a Tab keyup follows the manual completion query', async() => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === TauriCommand.GetKeymaps) {
        return Promise.resolve(DEFAULT_KEYMAPS)
      }

      if (cmd === TauriCommand.CompletionQuery) {
        return Promise.resolve({
          requestId: 'completion-1',
          sourceId: 'skills',
          replacementRange: { start: 0, end: 2 },
          items: [
            {
              label: 'git-commit',
              kind: CompletionKind.Skill,
              replacement: { range: { start: 0, end: 2 }, text: '#{hyprpilot://skills/git-commit}' }
            },
            {
              label: 'git-branch',
              kind: CompletionKind.Skill,
              replacement: { range: { start: 0, end: 2 }, text: '#{hyprpilot://skills/git-branch}' }
            }
          ]
        })
      }

      return Promise.resolve(undefined)
    })
    const wrapper = mount(ChatComposer, { attachTo: document.body })
    const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')

    await textarea.setValue('#g')
    textarea.element.setSelectionRange(2, 2)
    textarea.element.focus()
    await textarea.trigger('keydown', { key: ' ', ctrlKey: true })
    await new Promise((resolve) => setTimeout(resolve, 0))
    await wrapper.vm.$nextTick()
    await textarea.trigger('keyup', { key: 'Tab' })
    await wrapper.vm.$nextTick()

    expect(document.body.querySelector('.completion-popover-wrap')).not.toBeNull()
    expect(document.body.querySelector('[data-active="true"]')?.textContent).toContain('git-commit')

    wrapper.unmount()
  })

  it('swallows closed Tab without opening completion', async() => {
    const wrapper = mount(ChatComposer, { attachTo: document.body })
    const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')

    textarea.element.value = '#g'
    textarea.element.setSelectionRange(2, 2)
    textarea.element.focus()
    await new Promise((resolve) => setTimeout(resolve, 0))
    invokeMock.mockClear()

    const tab = new KeyboardEvent('keydown', {
      key: 'Tab', bubbles: true, cancelable: true
    })

    textarea.element.dispatchEvent(tab)
    await new Promise((resolve) => setTimeout(resolve, 0))
    await wrapper.vm.$nextTick()

    expect(tab.defaultPrevented).toBe(true)
    expect(invokeMock).not.toHaveBeenCalledWith(TauriCommand.CompletionQuery, expect.anything())
    expect(document.body.querySelector('.completion-popover-wrap')).toBeNull()

    const shiftTab = new KeyboardEvent('keydown', {
      key: 'Tab',
      shiftKey: true,
      bubbles: true,
      cancelable: true
    })

    textarea.element.dispatchEvent(shiftTab)
    await new Promise((resolve) => setTimeout(resolve, 0))
    await wrapper.vm.$nextTick()

    expect(shiftTab.defaultPrevented).toBe(true)
    expect(invokeMock).not.toHaveBeenCalledWith(TauriCommand.CompletionQuery, expect.anything())
    expect(document.body.querySelector('.completion-popover-wrap')).toBeNull()

    wrapper.unmount()
  })

  it('Enter submits; Shift+Enter does not', async() => {
    const wrapper = mount(ChatComposer, { attachTo: document.body })
    const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')

    await textarea.setValue('hi')
    textarea.element.focus()

    await textarea.trigger('keydown', { key: 'Enter', shiftKey: true })
    expect(wrapper.emitted('submit')).toBeUndefined()

    await textarea.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('submit')?.[0]).toBeDefined()
  })

  it('Enter on a coarse-pointer device falls through to newline, never submits', async() => {
    // Flip `matchMedia` to report a coarse pointer BEFORE mount so the
    // composable's snapshot captures the mobile shape. The captain's
    // bug: on a phone, Enter sent the message — no Shift on a soft
    // keyboard, so the buffer was stuck single-line.
    const originalMatchMedia = window.matchMedia

    window.matchMedia = vi.fn().mockImplementation((query: string) => ({
      matches: query.includes('coarse'),
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn().mockReturnValue(false)
    }))

    try {
      const wrapper = mount(ChatComposer, { attachTo: document.body })
      const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')

      await textarea.setValue('hi')
      textarea.element.focus()
      await textarea.trigger('keydown', { key: 'Enter' })
      expect(wrapper.emitted('submit')).toBeUndefined()

      // The submit button is still the explicit submit path on mobile.
      await wrapper.trigger('submit')
      expect(wrapper.emitted('submit')?.[0]).toBeDefined()
    } finally {
      window.matchMedia = originalMatchMedia
    }
  })
})
