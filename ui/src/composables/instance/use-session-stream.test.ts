import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ToastTone } from '@components'
import {
  useActiveInstance,
  resetPermissions,
  usePermissions,
  startSessionStream,
  resetStream,
  useStream,
  resetTerminals,
  useTerminals,
  clearToasts,
  useToasts,
  resetTools,
  useTools,
  resetTranscript,
  useTranscript
} from '@composables'
import { InstanceState, TauriEvent } from '@ipc'

const FMT = {
  title: 'bash',
  fields: []
}

type Handler = (payload: { payload: unknown }) => void

const { handlers, unlisten } = vi.hoisted(() => ({
  handlers: new Map<string, Handler>(),
  unlisten: vi.fn()
}))

// Mock `@ipc/bridge` directly — `@ipc` is a barrel and the
// `listen` re-export binds at evaluation time, so mocking `@ipc`
// alone leaves bridge.ts's `tauriListen` pinned through. Same
// pattern as `use-home-dir.test.ts`.
vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: vi.fn(),
  listen: (event: string, cb: Handler) => {
    handlers.set(event, cb)

    return Promise.resolve(unlisten)
  }
}))

function emit(event: string, payload: unknown) {
  const cb = handlers.get(event)

  if (!cb) {
    throw new Error(`no listener registered for ${event}`)
  }
  cb({ payload })
}

beforeEach(() => {
  handlers.clear()
  unlisten.mockReset()
  useActiveInstance().id.value = undefined
  clearToasts()
  resetTranscript('A')
  resetTranscript('B')
  resetStream('A')
  resetStream('B')
  resetTools('A')
  resetTools('B')
  resetTerminals('A')
  resetTerminals('B')
  resetPermissions('A')
  resetPermissions('B')
})

describe('useSessionStream', () => {
  it('subscribes to every transcript / lifecycle / metadata event channel', async() => {
    await startSessionStream()
    expect([...handlers.keys()].sort()).toEqual(
      [
        TauriEvent.AcpCurrentModeUpdate,
        TauriEvent.AcpInstanceMeta,
        TauriEvent.AcpInstanceRenamed,
        TauriEvent.AcpInstanceState,
        TauriEvent.AcpPermissionRequest,
        TauriEvent.AcpSessionInfoUpdate,
        TauriEvent.AcpTerminal,
        TauriEvent.AcpTranscript,
        TauriEvent.AcpTurnEnded,
        TauriEvent.AcpTurnStarted,
        TauriEvent.ComposerDraftAppend
      ].sort()
    )
  })

  it('routes acp:permission-request events into the per-instance usePermissions store', async() => {
    await startSessionStream()

    emit(TauriEvent.AcpPermissionRequest, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'echo hi',
      options: [
        {
          optionId: 'allow',
          name: 'Allow',
          kind: 'y'
        }
      ],
      formatted: FMT
    })

    const queue = usePermissions('A').rowQueue.value

    expect(queue).toHaveLength(1)
    expect(queue[0]?.request.requestId).toBe('req-1')
    expect(queue[0]?.request.toolName).toBe('bash')
  })

  it('keeps concurrent permission prompts distinct when Rust emits unique request_ids', async() => {
    await startSessionStream()

    emit(TauriEvent.AcpPermissionRequest, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'execute',
      args: 'ls',
      options: [
        {
          optionId: 'allow',
          name: 'Allow',
          kind: 'y'
        }
      ],
      formatted: FMT
    })
    emit(TauriEvent.AcpPermissionRequest, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      requestId: 'req-2',
      tool: 'bash',
      kind: 'execute',
      args: 'pwd',
      options: [
        {
          optionId: 'deny',
          name: 'Deny',
          kind: 'n'
        }
      ],
      formatted: FMT
    })

    const queue = usePermissions('A').rowQueue.value

    expect(queue).toHaveLength(2)
    expect(new Set(queue.map((v) => v.request.requestId)).size).toBe(2)
    expect(queue.map((v) => v.request.requestId).sort()).toEqual(['req-1', 'req-2'])
  })

  it('routes acp:transcript events to the per-instance transcript store', async() => {
    await startSessionStream()

    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      item: { kind: 'user_text', text: 'hi' }
    })
    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-b',
      instanceId: 'B',
      item: { kind: 'user_text', text: 'yo' }
    })

    const a = useTranscript('A').turns.value
    const b = useTranscript('B').turns.value

    expect(a.map((t) => t.text)).toEqual(['hi'])
    expect(b.map((t) => t.text)).toEqual(['yo'])
  })

  it('routes thought / plan / tool_call updates to their respective stores', async() => {
    await startSessionStream()

    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      item: { kind: 'agent_thought', text: 'planning' }
    })
    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      item: { kind: 'plan', steps: [{ content: 'step-1' }] }
    })
    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      item: {
        kind: 'tool_call',
        id: 'tc-1',
        title: 'bash',
        toolKind: 'bash',
        state: 'running',
        rawInput: { command: 'echo hi' },
        content: [{ kind: 'text', text: 'hi\n' }]
      }
    })

    const stream = useStream('A').items.value

    expect(stream).toHaveLength(2)

    const tools = useTools('A').calls.value

    expect(tools).toHaveLength(1)
    expect(tools[0]?.toolCallId).toBe('tc-1')
  })

  it('routes acp:terminal output and exit chunks to useTerminals', async() => {
    await startSessionStream()

    emit(TauriEvent.AcpTerminal, {
      agentId: 'a',
      instanceId: 'A',
      sessionId: 's-a',
      terminalId: 'term-1',
      chunk: {
        kind: 'output',
        stream: 'stdout',
        data: 'line 1\n'
      }
    })
    emit(TauriEvent.AcpTerminal, {
      agentId: 'a',
      instanceId: 'A',
      sessionId: 's-a',
      terminalId: 'term-1',
      chunk: {
        kind: 'output',
        stream: 'stdout',
        data: 'line 2\n'
      }
    })
    emit(TauriEvent.AcpTerminal, {
      agentId: 'a',
      instanceId: 'A',
      sessionId: 's-a',
      terminalId: 'term-1',
      chunk: { kind: 'exit', exitCode: 0 }
    })

    const entry = useTerminals('A').byId('term-1').value

    expect(entry?.output).toBe('line 1\nline 2\n')
    expect(entry?.running).toBe(false)
    expect(entry?.exitCode).toBe(0)
  })

  it('promotes the first running instance to active via useActiveInstance', async() => {
    await startSessionStream()
    const { id } = useActiveInstance()

    expect(id.value).toBeUndefined()

    emit(TauriEvent.AcpInstanceState, {
      agentId: 'a',
      instanceId: 'A',
      state: InstanceState.Starting
    })
    expect(id.value).toBeUndefined()

    emit(TauriEvent.AcpInstanceState, {
      agentId: 'a',
      instanceId: 'A',
      state: InstanceState.Running
    })
    expect(id.value).toBe('A')

    emit(TauriEvent.AcpInstanceState, {
      agentId: 'a',
      instanceId: 'B',
      state: InstanceState.Running
    })
    expect(id.value).toBe('A')
  })

  it('unsubscribes every channel when the returned stop fn runs', async() => {
    const stop = await startSessionStream()

    stop()
    expect(unlisten).toHaveBeenCalledTimes(11)
  })

  it('pushes an ok toast when acp:instance-state transitions to running', async() => {
    await startSessionStream()

    emit(TauriEvent.AcpInstanceState, {
      agentId: 'a',
      instanceId: 'A',
      state: InstanceState.Running
    })

    const { entries } = useToasts()

    expect(entries.value).toHaveLength(1)
    expect(entries.value[0]?.tone).toBe(ToastTone.Ok)
    expect(entries.value[0]?.body).toBe('session started')
  })

  it('pushes a warn toast when acp:instance-state ends after running — not after starting', async() => {
    await startSessionStream()

    // Ended after running → toast
    emit(TauriEvent.AcpInstanceState, {
      agentId: 'a',
      instanceId: 'A',
      state: InstanceState.Running
    })
    emit(TauriEvent.AcpInstanceState, {
      agentId: 'a',
      instanceId: 'A',
      state: InstanceState.Ended
    })

    const { entries } = useToasts()
    const messages = entries.value.map((t) => t.body)

    expect(messages).toContain('session ended')

    // Clear and try: ended without ever running → no "session ended" toast
    clearToasts()
    emit(TauriEvent.AcpInstanceState, {
      agentId: 'b',
      instanceId: 'B',
      state: InstanceState.Ended
    })

    expect(entries.value.find((t) => t.body === 'session ended')).toBeUndefined()
  })

  it('clears priorState map on stop so a new startSessionStream begins fresh', async() => {
    const stop = await startSessionStream()

    emit(TauriEvent.AcpInstanceState, {
      agentId: 'a',
      instanceId: 'A',
      state: InstanceState.Running
    })
    stop()
    clearToasts()

    // New stream should not carry prior state from previous stream
    await startSessionStream()
    emit(TauriEvent.AcpInstanceState, {
      agentId: 'a',
      instanceId: 'A',
      state: InstanceState.Ended
    })

    const { entries } = useToasts()

    expect(entries.value.find((t) => t.body === 'session ended')).toBeUndefined()
  })
})
