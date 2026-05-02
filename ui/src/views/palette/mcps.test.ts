import { beforeEach, describe, expect, it, vi } from 'vitest'

import { openMcpsLeaf } from './mcps'
import { __resetPaletteStackForTests, PaletteMode, usePalette, clearToasts } from '@composables'
import { TauriCommand } from '@ipc'

type Handler = (payload: { payload: unknown }) => void
type InvokeArgs = Record<string, unknown>

interface InvokeCall {
  cmd: TauriCommand
  args?: InvokeArgs
}

const { handlers, unlisten, invokeCalls, invokeImpl } = vi.hoisted(() => ({
  handlers: new Map<string, Handler>(),
  unlisten: vi.fn(),
  invokeCalls: [] as InvokeCall[],
  invokeImpl: { fn: undefined as ((cmd: TauriCommand, args?: InvokeArgs) => Promise<unknown>) | undefined }
}))

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: vi.fn((cmd: TauriCommand, args?: InvokeArgs) => {
    invokeCalls.push({ cmd, args })

    if (invokeImpl.fn) {
      return invokeImpl.fn(cmd, args)
    }

    return Promise.resolve({ mcps: [] })
  }),
  listen: (event: string, cb: Handler) => {
    handlers.set(event, cb)

    return Promise.resolve(unlisten)
  }
}))

beforeEach(() => {
  handlers.clear()
  invokeCalls.length = 0
  invokeImpl.fn = undefined
  unlisten.mockReset()
  __resetPaletteStackForTests()
  clearToasts()
})

describe('openMcpsLeaf', () => {
  it('lists with the given instanceId and pushes a readonly select-mode palette spec with preview', async() => {
    invokeImpl.fn = async() => ({
      mcps: [
        {
          name: 'filesystem',
          raw: { command: 'npx', args: ['-y', 'fs'] },
          hyprpilot: { autoAcceptTools: ['read_*'], autoRejectTools: [] },
          source: '/etc/mcps/base.json'
        },
        {
          name: 'github',
          raw: { command: 'uvx', args: ['mcp-server-github'] },
          hyprpilot: { autoAcceptTools: [], autoRejectTools: [] },
          source: '/etc/mcps/base.json'
        }
      ]
    })

    await openMcpsLeaf({ instanceId: 'inst-1' })

    expect(invokeCalls).toHaveLength(1)
    expect(invokeCalls[0]?.cmd).toBe(TauriCommand.McpsList)
    expect(invokeCalls[0]?.args).toEqual({ instanceId: 'inst-1' })

    const { stack } = usePalette()

    expect(stack.value).toHaveLength(1)
    const spec = stack.value[0]!

    expect(spec.mode).toBe(PaletteMode.Select)
    expect(spec.title).toBe('mcps')
    expect(spec.entries.map((e) => e.id)).toEqual(['filesystem', 'github'])
    // No preseedActive — readonly palette has no toggling.
    expect(spec.preseedActive).toBeUndefined()
    // Preview component + items propagated.
    expect(spec.preview).toBeDefined()
    expect(spec.preview?.props?.items).toBeInstanceOf(Array)
  })

  it('commit is a no-op (readonly view)', async() => {
    invokeImpl.fn = async() => ({
      mcps: [
        {
          name: 'filesystem',
          raw: { command: 'npx' },
          hyprpilot: { autoAcceptTools: [], autoRejectTools: [] },
          source: '/etc/mcps/base.json'
        }
      ]
    })

    await openMcpsLeaf({ instanceId: 'inst-1' })
    invokeCalls.length = 0

    const { stack } = usePalette()
    const spec = stack.value[0]!

    void spec.onCommit([{ id: 'filesystem', name: 'filesystem' }])

    // Readonly: commit fires no further IPC calls.
    expect(invokeCalls).toHaveLength(0)
  })

  it('omits instanceId when not provided', async() => {
    invokeImpl.fn = async() => ({ mcps: [] })

    await openMcpsLeaf()

    expect(invokeCalls).toHaveLength(1)
    expect(invokeCalls[0]?.args).toEqual({ instanceId: undefined })
  })
})
