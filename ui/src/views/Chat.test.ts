import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { Modifier, TauriCommand } from '@ipc'

const { invoke, listeners, unlisten } = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (payload: { payload: unknown }) => void>(),
  unlisten: vi.fn()
}))

vi.mock('@ipc/bridge', async () => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: (event: string, cb: (payload: { payload: unknown }) => void) => {
    listeners.set(event, cb)

    return Promise.resolve(unlisten)
  },
  getProfiles: () => Promise.resolve([]),
  listSessions: () => Promise.resolve([]),
  loadSession: () => Promise.resolve()
}))

import { useActiveInstance } from '@composables/use-active-instance'
import { __resetKeymapsForTests, loadKeymaps } from '@composables/use-keymaps'
import { pushPermissionRequest, resetPermissions } from '@composables/use-permissions'

import Chat from './Overlay.vue'

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

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await Promise.resolve()
  }
}

beforeEach(async () => {
  invoke.mockReset()
  listeners.clear()
  unlisten.mockReset()
  resetPermissions('A')
  resetPermissions('B')
  useActiveInstance().id.value = 'A'
  __resetKeymapsForTests()
  // Pre-populate the keymap cache — onMounted in Overlay.vue bails
  // early when `useKeymaps().keymaps.value` is undefined.
  invoke.mockImplementation((command: string) => {
    if (command === TauriCommand.GetKeymaps) {
      return Promise.resolve(DEFAULT_KEYMAPS)
    }
    return Promise.resolve(undefined)
  })
  await loadKeymaps()
  invoke.mockReset()
})

describe('Chat.vue — permission wiring', () => {
  it('renders pending prompts from usePermissions and dispatches permission_reply on allow click', async () => {
    pushPermissionRequest('A', 's-a', {
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'echo hi',
      options: [{ optionId: 'allow', name: 'Allow', kind: 'y' }]
    })
    invoke.mockResolvedValue(undefined)

    const wrapper = mount(Chat, { attachTo: document.body })
    await flushMicrotasks()

    const stack = wrapper.get('[data-testid="permission-stack"]')
    expect(stack.text()).toContain('1 pending')

    const allowButton = stack.findAll('button').find((b) => b.text().includes('allow'))!
    await allowButton.trigger('click')
    await flushMicrotasks()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'allow'
    })
    expect(wrapper.find('[data-testid="permission-stack"]').exists()).toBe(false)
    wrapper.unmount()
  })

  it('dispatches deny via keyboard `d` when no input has focus', async () => {
    pushPermissionRequest('A', 's-a', {
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'rm -rf /',
      options: [{ optionId: 'deny', name: 'Deny', kind: 'n' }]
    })
    invoke.mockResolvedValue(undefined)

    const wrapper = mount(Chat, { attachTo: document.body })
    await flushMicrotasks()

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'd', bubbles: true }))
    await flushMicrotasks()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'deny'
    })
    wrapper.unmount()
  })

  it('dispatches allow via keyboard `a` when no input has focus', async () => {
    pushPermissionRequest('A', 's-a', {
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'ls',
      options: [{ optionId: 'allow', name: 'Allow', kind: 'y' }]
    })
    invoke.mockResolvedValue(undefined)

    const wrapper = mount(Chat, { attachTo: document.body })
    await flushMicrotasks()

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true }))
    await flushMicrotasks()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'allow'
    })
    wrapper.unmount()
  })

  it('does not dispatch when the composer textarea has focus', async () => {
    pushPermissionRequest('A', 's-a', {
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'ls',
      options: [{ optionId: 'allow', name: 'Allow', kind: 'y' }]
    })

    const wrapper = mount(Chat, { attachTo: document.body })
    await flushMicrotasks()

    const textarea = wrapper.find('textarea')
    expect(textarea.exists()).toBe(true)
    textarea.element.focus()

    // Dispatch through the textarea so event.target points at the composer.
    textarea.element.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true, cancelable: true }))
    await flushMicrotasks()

    expect(invoke).not.toHaveBeenCalledWith(TauriCommand.PermissionReply, expect.anything())
    wrapper.unmount()
  })

  it('surfaces reply failure as an error toast', async () => {
    pushPermissionRequest('A', 's-a', {
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'ls',
      options: [{ optionId: 'allow', name: 'Allow', kind: 'y' }]
    })
    invoke.mockRejectedValue(new Error('permission_reply not implemented (K-245)'))

    const wrapper = mount(Chat, { attachTo: document.body })
    await flushMicrotasks()

    const allowButton = wrapper.findAll('button').find((b) => b.text().includes('allow'))!
    await allowButton.trigger('click')
    await flushMicrotasks()

    // K-254: errors route through the toast stack, not the inline chat-err band.
    const toastStack = wrapper.find('.toast-stack')
    expect(toastStack.exists()).toBe(true)
    expect(toastStack.text()).toContain('allow failed')
    wrapper.unmount()
  })

  it('ignores keyboard shortcuts when modifier keys are held', async () => {
    pushPermissionRequest('A', 's-a', {
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'ls',
      options: [{ optionId: 'allow', name: 'Allow', kind: 'y' }]
    })

    const wrapper = mount(Chat, { attachTo: document.body })
    await flushMicrotasks()

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', ctrlKey: true, bubbles: true }))
    await flushMicrotasks()

    expect(invoke).not.toHaveBeenCalledWith(TauriCommand.PermissionReply, expect.anything())
    wrapper.unmount()
  })
})
