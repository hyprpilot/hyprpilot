import { beforeEach, describe, expect, it, vi } from 'vitest'

import { InstanceState, TauriEvent } from '@ipc'

import { ToastTone } from '@components/types'

import { useActiveInstance } from '@composables/use-active-instance'
import { resetPermissions, usePermissions } from '@composables/use-permissions'
import { startSessionStream } from '@composables/use-session-stream'
import { resetStream, useStream } from '@composables/use-stream'
import { resetTerminals, useTerminals } from '@composables/use-terminals'
import { clearToasts, useToasts } from '@composables/use-toasts'
import { resetTools, useTools } from '@composables/use-tools'
import { resetTranscript, useTranscript } from '@composables/use-transcript'

type Handler = (payload: { payload: unknown }) => void

const handlers = new Map<string, Handler>()
const unlisten = vi.fn()

vi.mock('@ipc', async () => {
  const actual = await vi.importActual<typeof import('@ipc')>('@ipc')
  return {
    ...actual,
    invoke: vi.fn(),
    listen: (event: string, cb: Handler) => {
      handlers.set(event, cb)

      return Promise.resolve(unlisten)
    }
  }
})

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
  it('subscribes to acp:transcript + acp:instance-state + acp:permission-request', async () => {
    await startSessionStream()
    expect([...handlers.keys()].sort()).toEqual([TauriEvent.AcpInstanceState, TauriEvent.AcpPermissionRequest, TauriEvent.AcpTranscript])
  })

  it('routes acp:permission-request events into the per-instance usePermissions store', async () => {
    await startSessionStream()

    emit(TauriEvent.AcpPermissionRequest, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'echo hi',
      options: [{ optionId: 'allow', name: 'Allow', kind: 'y' }]
    })

    const pending = usePermissions('A').pending.value
    expect(pending).toHaveLength(1)
    expect(pending[0]?.requestId).toBe('req-1')
    expect(pending[0]?.tool).toBe('bash')
  })

  it('keeps concurrent permission prompts distinct when Rust emits unique request_ids', async () => {
    await startSessionStream()

    emit(TauriEvent.AcpPermissionRequest, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'execute',
      args: 'ls',
      options: [{ optionId: 'allow', name: 'Allow', kind: 'y' }]
    })
    emit(TauriEvent.AcpPermissionRequest, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      requestId: 'req-2',
      tool: 'bash',
      kind: 'execute',
      args: 'pwd',
      options: [{ optionId: 'deny', name: 'Deny', kind: 'n' }]
    })

    const pending = usePermissions('A').pending.value
    expect(pending).toHaveLength(2)
    expect(new Set(pending.map((p) => p.requestId)).size).toBe(2)
    expect(pending.map((p) => p.args).sort()).toEqual(['ls', 'pwd'])
  })

  it('routes acp:transcript events to the per-instance transcript store', async () => {
    await startSessionStream()

    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      update: { sessionUpdate: 'user_message_chunk', content: { type: 'text', text: 'hi' } }
    })
    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-b',
      instanceId: 'B',
      update: { sessionUpdate: 'user_message_chunk', content: { type: 'text', text: 'yo' } }
    })

    const a = useTranscript('A').turns.value
    const b = useTranscript('B').turns.value
    expect(a.map((t) => t.text)).toEqual(['hi'])
    expect(b.map((t) => t.text)).toEqual(['yo'])
  })

  it('routes thought / plan / tool_call / terminal chunks to their respective stores', async () => {
    await startSessionStream()

    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      update: { sessionUpdate: 'agent_thought_chunk', content: { text: 'planning' } }
    })
    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      update: { sessionUpdate: 'plan', entries: [{ content: 'step-1' }] }
    })
    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      update: {
        sessionUpdate: 'tool_call',
        toolCallId: 'tc-1',
        title: 'bash',
        kind: 'bash',
        rawInput: { command: 'echo hi' },
        content: [{ type: 'text', text: 'hi\n' }]
      }
    })

    const stream = useStream('A').items.value
    expect(stream).toHaveLength(2)

    const tools = useTools('A').calls.value
    expect(tools).toHaveLength(1)
    expect(tools[0]?.toolCallId).toBe('tc-1')

    const term = useTerminals('A').streams.value
    expect(term['tc-1']?.stdout).toBe('hi\n')
    expect(term['tc-1']?.command).toBe('echo hi')
  })

  it('promotes the first running instance to active via useActiveInstance', async () => {
    await startSessionStream()
    const { id } = useActiveInstance()
    expect(id.value).toBeUndefined()

    emit(TauriEvent.AcpInstanceState, { agentId: 'a', instanceId: 'A', state: InstanceState.Starting })
    expect(id.value).toBeUndefined()

    emit(TauriEvent.AcpInstanceState, { agentId: 'a', instanceId: 'A', state: InstanceState.Running })
    expect(id.value).toBe('A')

    emit(TauriEvent.AcpInstanceState, { agentId: 'a', instanceId: 'B', state: InstanceState.Running })
    expect(id.value).toBe('A')
  })

  it('keeps routing stdout on tool_call_update chunks that omit `kind` after the initial tool_call', async () => {
    await startSessionStream()

    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      update: {
        sessionUpdate: 'tool_call',
        toolCallId: 'tc-bash',
        title: 'bash',
        kind: 'bash',
        rawInput: { command: 'tail -f log' },
        content: [{ type: 'text', text: 'line 1\n' }]
      }
    })
    emit(TauriEvent.AcpTranscript, {
      agentId: 'a',
      sessionId: 's-a',
      instanceId: 'A',
      update: {
        sessionUpdate: 'tool_call_update',
        toolCallId: 'tc-bash',
        content: [{ type: 'text', text: 'line 2\n' }]
      }
    })

    const term = useTerminals('A').streams.value
    expect(term['tc-bash']?.stdout).toBe('line 1\nline 2\n')
  })

  it('unsubscribes every channel when the returned stop fn runs', async () => {
    const stop = await startSessionStream()
    stop()
    expect(unlisten).toHaveBeenCalledTimes(3)
  })

  it('pushes an ok toast when acp:instance-state transitions to running', async () => {
    await startSessionStream()

    emit(TauriEvent.AcpInstanceState, { agentId: 'a', instanceId: 'A', state: InstanceState.Running })

    const { entries } = useToasts()
    expect(entries.value).toHaveLength(1)
    expect(entries.value[0]?.tone).toBe(ToastTone.Ok)
    expect(entries.value[0]?.message).toBe('session started')
  })

  it('pushes a warn toast when acp:instance-state ends after running — not after starting', async () => {
    await startSessionStream()

    // Ended after running → toast
    emit(TauriEvent.AcpInstanceState, { agentId: 'a', instanceId: 'A', state: InstanceState.Running })
    emit(TauriEvent.AcpInstanceState, { agentId: 'a', instanceId: 'A', state: InstanceState.Ended })

    const { entries } = useToasts()
    const messages = entries.value.map((t) => t.message)
    expect(messages).toContain('session ended')

    // Clear and try: ended without ever running → no "session ended" toast
    clearToasts()
    emit(TauriEvent.AcpInstanceState, { agentId: 'b', instanceId: 'B', state: InstanceState.Ended })

    expect(entries.value.find((t) => t.message === 'session ended')).toBeUndefined()
  })

  it('clears priorState map on stop so a new startSessionStream begins fresh', async () => {
    const stop = await startSessionStream()
    emit(TauriEvent.AcpInstanceState, { agentId: 'a', instanceId: 'A', state: InstanceState.Running })
    stop()
    clearToasts()

    // New stream should not carry prior state from previous stream
    await startSessionStream()
    emit(TauriEvent.AcpInstanceState, { agentId: 'a', instanceId: 'A', state: InstanceState.Ended })

    const { entries } = useToasts()
    expect(entries.value.find((t) => t.message === 'session ended')).toBeUndefined()
  })
})
