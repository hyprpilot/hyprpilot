import { beforeEach, describe, expect, it, vi } from 'vitest'

import { InstanceState, TauriCommand, TauriEvent } from '@ipc'

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

vi.mock('@ipc/bridge', async () => ({
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

import { __resetPaletteStackForTests, type PaletteEntry, PaletteMode, usePalette } from '@composables'
import { clearToasts, useToasts } from '@composables'

import { mcpsDiffersFromBaseline, openMcpsLeaf } from './mcps'

function emit(event: string, payload: unknown): void {
  const cb = handlers.get(event)
  if (!cb) {
    throw new Error(`no listener registered for ${event}`)
  }
  cb({ payload })
}

async function flushAsync(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
}

beforeEach(() => {
  handlers.clear()
  invokeCalls.length = 0
  invokeImpl.fn = undefined
  unlisten.mockReset()
  __resetPaletteStackForTests()
  clearToasts()
})

describe('mcpsDiffersFromBaseline', () => {
  it('returns false when sets are equal', () => {
    expect(mcpsDiffersFromBaseline(new Set(['a', 'b']), new Set(['b', 'a']))).toBe(false)
  })

  it('returns true when sizes differ', () => {
    expect(mcpsDiffersFromBaseline(new Set(['a']), new Set(['a', 'b']))).toBe(true)
  })

  it('returns true when same size but different membership', () => {
    expect(mcpsDiffersFromBaseline(new Set(['a', 'b']), new Set(['a', 'c']))).toBe(true)
  })
})

describe('openMcpsLeaf', () => {
  it('lists with the given instanceId, pre-ticks enabled rows, and pushes a multi-select palette spec', async () => {
    invokeImpl.fn = async () =>
      ({
        mcps: [
          { name: 'fs', command: 'uvx', enabled: true },
          { name: 'rg', command: 'rg', enabled: false }
        ]
      })

    await openMcpsLeaf({ instanceId: 'inst-1' })

    expect(invokeCalls).toHaveLength(1)
    expect(invokeCalls[0]?.cmd).toBe(TauriCommand.McpsList)
    expect(invokeCalls[0]?.args).toEqual({ instanceId: 'inst-1' })

    const { stack } = usePalette()
    expect(stack.value).toHaveLength(1)
    const spec = stack.value[0]!
    expect(spec.mode).toBe(PaletteMode.MultiSelect)
    expect(spec.title).toBe('mcps')
    expect(spec.entries.map((e) => e.id)).toEqual(['fs', 'rg'])
    expect(spec.preseedActive?.map((e) => e.id)).toEqual(['fs'])
  })

  it('commit with no diff against baseline does NOT call mcps_set', async () => {
    invokeImpl.fn = async () =>
      ({
        mcps: [
          { name: 'fs', command: 'uvx', enabled: true },
          { name: 'rg', command: 'rg', enabled: false }
        ]
      })

    await openMcpsLeaf({ instanceId: 'inst-1' })
    invokeCalls.length = 0

    const { stack } = usePalette()
    const spec = stack.value[0]!
    // Replay the same `enabled` set the baseline ticked.
    const picks: PaletteEntry[] = [{ id: 'fs', name: 'fs' }]
    void spec.onCommit(picks)
    await flushAsync()

    expect(invokeCalls).toHaveLength(0)
  })

  it('commit with diff calls mcps_set with the new ticked slugs and pushes a "switching" toast', async () => {
    invokeImpl.fn = async (cmd: TauriCommand) => {
      if (cmd === TauriCommand.McpsList) {
        return {
          mcps: [
            { name: 'fs', command: 'uvx', enabled: true },
            { name: 'rg', command: 'rg', enabled: false }
          ]
        }
      }

      return { restarted: true }
    }

    await openMcpsLeaf({ instanceId: 'inst-1', agentLabel: 'claude-code' })
    invokeCalls.length = 0

    const { stack } = usePalette()
    const spec = stack.value[0]!
    // Add `rg` to the baseline (`fs`).
    const picks: PaletteEntry[] = [
      { id: 'fs', name: 'fs' },
      { id: 'rg', name: 'rg' }
    ]
    void spec.onCommit(picks)
    await flushAsync()

    expect(invokeCalls).toHaveLength(1)
    expect(invokeCalls[0]?.cmd).toBe(TauriCommand.McpsSet)
    expect(invokeCalls[0]?.args).toMatchObject({ instanceId: 'inst-1' })
    expect(new Set((invokeCalls[0]?.args?.enabled as string[]) ?? [])).toEqual(new Set(['fs', 'rg']))

    const { entries } = useToasts()
    const messages = entries.value.map((t) => t.body).filter((b): b is string => typeof b === 'string')
    expect(messages.some((m) => m.includes('switching MCPs') && m.includes('claude-code'))).toBe(true)
  })

  it('on `running` for the addressed instance pushes a ready toast', async () => {
    invokeImpl.fn = async (cmd: TauriCommand) => {
      if (cmd === TauriCommand.McpsList) {
        return { mcps: [{ name: 'fs', command: 'uvx', enabled: false }] }
      }

      return { restarted: true }
    }

    await openMcpsLeaf({ instanceId: 'inst-1', agentLabel: 'claude-code' })
    const { stack } = usePalette()
    const spec = stack.value[0]!
    void spec.onCommit([{ id: 'fs', name: 'fs' }])
    await flushAsync()
    await flushAsync()

    emit(TauriEvent.AcpInstanceState, { agentId: 'a', instanceId: 'inst-1', state: InstanceState.Running })
    await flushAsync()

    const { entries } = useToasts()
    const messages = entries.value.map((t) => t.body).filter((b): b is string => typeof b === 'string')
    expect(messages.some((m) => m.includes('ready'))).toBe(true)
  })

  it('on `running` for a different instance does NOT push a ready toast', async () => {
    invokeImpl.fn = async (cmd: TauriCommand) => {
      if (cmd === TauriCommand.McpsList) {
        return { mcps: [{ name: 'fs', command: 'uvx', enabled: false }] }
      }

      return { restarted: true }
    }

    await openMcpsLeaf({ instanceId: 'inst-1', agentLabel: 'claude-code' })
    const { stack } = usePalette()
    const spec = stack.value[0]!
    void spec.onCommit([{ id: 'fs', name: 'fs' }])
    await flushAsync()
    await flushAsync()

    emit(TauriEvent.AcpInstanceState, { agentId: 'a', instanceId: 'OTHER', state: InstanceState.Running })
    await flushAsync()

    const { entries } = useToasts()
    expect(entries.value.some((t) => typeof t.body === 'string' && t.body.includes('ready'))).toBe(false)
  })
})
