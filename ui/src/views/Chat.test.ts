import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import Chat from './Overlay.vue'
import { useActiveInstance, __resetKeymapsForTests, loadKeymaps, pushPermissionRequest, resetPermissions, clearToasts, useToasts } from '@composables'
import { Modifier, TauriCommand } from '@ipc'

const { invoke, listeners, unlisten } = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (payload: { payload: unknown }) => void>(),
  unlisten: vi.fn()
}))

vi.mock('@ipc/bridge', async() => ({
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

const DEFAULT_KEYMAPS = {
  chat: {
    submit: { modifiers: [], key: 'enter' },
    newline: { modifiers: [Modifier.Shift], key: 'enter' },
    cancel_turn: { modifiers: [Modifier.Ctrl], key: 'c' }
  },
  approvals: {
    allow: { modifiers: [Modifier.Ctrl], key: 'g' },
    deny: { modifiers: [Modifier.Ctrl], key: 'r' }
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
  transcript: {},
  window: {
    toggle: { modifiers: [Modifier.Ctrl], key: 'q' }
  },
  queue: {
    send: { modifiers: [Modifier.Ctrl], key: 'enter' },
    drop: { modifiers: [Modifier.Ctrl], key: 'backspace' }
  }
}

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await Promise.resolve()
  }
}

beforeEach(async() => {
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

// Realistic ACP option set: agent typically offers all four kinds
// (`allow_once`, `allow_always`, `reject_once`, `reject_always`).
// Tests use a stable subset and assert on the typed `kind` lookup
// (keybind picks the first option matching `allow*` / `reject*`).
const SAMPLE_OPTIONS = [
  {
    optionId: 'allow-once-id', name: 'Allow once', kind: 'allow_once'
  },
  {
    optionId: 'allow-always-id', name: 'Allow always', kind: 'allow_always'
  },
  {
    optionId: 'reject-once-id', name: 'Reject once', kind: 'reject_once'
  }
]

const FMT = {
  title: 'bash',
  fields: []
}

describe('Chat.vue — permission wiring', () => {
  it('renders pending prompts from usePermissions and dispatches permission_reply on a button click', async() => {
    pushPermissionRequest('A', 's-a', {
      agentId: 'agent-A',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'echo hi',
      options: SAMPLE_OPTIONS,
      formatted: FMT
    })
    invoke.mockResolvedValue(undefined)

    const wrapper = mount(Chat, { attachTo: document.body })

    await flushMicrotasks()

    const stack = wrapper.get('[data-testid="permission-stack"]')

    expect(stack.text()).toContain('bash')

    // Button labels go through `change-case::sentenceCase` on the
    // agent-supplied `name` field. `'Allow once'` round-trips as-is.
    const allowButton = stack.find('button[aria-label="Allow once"]')

    await allowButton.trigger('click')
    await flushMicrotasks()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'allow-once-id'
    })
    expect(wrapper.find('[data-testid="permission-stack"]').exists()).toBe(false)
    wrapper.unmount()
  })

  it('dispatches the first reject_* option via Ctrl+R', async() => {
    pushPermissionRequest('A', 's-a', {
      agentId: 'agent-A',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'rm -rf /',
      options: SAMPLE_OPTIONS,
      formatted: FMT
    })
    invoke.mockResolvedValue(undefined)

    const wrapper = mount(Chat, { attachTo: document.body })

    await flushMicrotasks()

    document.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'r',
        ctrlKey: true,
        bubbles: true
      })
    )
    await flushMicrotasks()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'reject-once-id'
    })
    wrapper.unmount()
  })

  it('dispatches the first allow_* option via Ctrl+G', async() => {
    pushPermissionRequest('A', 's-a', {
      agentId: 'agent-A',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'ls',
      options: SAMPLE_OPTIONS,
      formatted: FMT
    })
    invoke.mockResolvedValue(undefined)

    const wrapper = mount(Chat, { attachTo: document.body })

    await flushMicrotasks()

    document.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'g',
        ctrlKey: true,
        bubbles: true
      })
    )
    await flushMicrotasks()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'allow-once-id'
    })
    wrapper.unmount()
  })

  it('dispatches Ctrl+G even when the composer textarea has focus', async() => {
    pushPermissionRequest('A', 's-a', {
      agentId: 'agent-A',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'ls',
      options: SAMPLE_OPTIONS,
      formatted: FMT
    })
    invoke.mockResolvedValue(undefined)

    const wrapper = mount(Chat, { attachTo: document.body })

    await flushMicrotasks()

    const textarea = wrapper.find('textarea')

    expect(textarea.exists()).toBe(true)
    textarea.element.focus()

    textarea.element.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'g',
        ctrlKey: true,
        bubbles: true,
        cancelable: true
      })
    )
    await flushMicrotasks()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'allow-once-id'
    })
    wrapper.unmount()
  })

  it('surfaces reply failure as an error toast', async() => {
    pushPermissionRequest('A', 's-a', {
      agentId: 'agent-A',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'ls',
      options: SAMPLE_OPTIONS,
      formatted: FMT
    })
    invoke.mockRejectedValue(new Error('permission_reply not implemented (K-245)'))

    const wrapper = mount(Chat, { attachTo: document.body })

    await flushMicrotasks()

    const allowButton = wrapper.find('button[aria-label="Allow once"]')

    await allowButton.trigger('click')
    await flushMicrotasks()

    // Errors route through the toast pipeline (vue-sonner) — assert via the
    // audit-log mirror since Sonner portals out of the wrapper.
    const messages = useToasts()
      .entries.value.map((t) => t.body)
      .filter((b): b is string => typeof b === 'string')

    expect(messages.some((m) => m.includes('permission reply failed'))).toBe(true)
    clearToasts()
    wrapper.unmount()
  })

  it('does not fire allow when the modifier is missing — plain `g` is just typing', async() => {
    pushPermissionRequest('A', 's-a', {
      agentId: 'agent-A',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'ls',
      options: SAMPLE_OPTIONS,
      formatted: FMT
    })

    const wrapper = mount(Chat, { attachTo: document.body })

    await flushMicrotasks()

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'g', bubbles: true }))
    await flushMicrotasks()

    expect(invoke).not.toHaveBeenCalledWith(TauriCommand.PermissionReply, expect.anything())
    wrapper.unmount()
  })
})
