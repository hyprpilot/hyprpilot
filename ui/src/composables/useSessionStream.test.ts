import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useActiveInstance } from '@composables/useActiveInstance'
import { resetPermissions, usePermissions } from '@composables/usePermissions'
import { InstanceState, startSessionStream } from '@composables/useSessionStream'
import { resetStream, useStream } from '@composables/useStream'
import { resetTerminals, useTerminals } from '@composables/useTerminals'
import { resetTools, useTools } from '@composables/useTools'
import { resetTranscript, useTranscript } from '@composables/useTranscript'

type Handler = (payload: { payload: unknown }) => void

const handlers = new Map<string, Handler>()
const unlisten = vi.fn()

vi.mock('@ipc', () => ({
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
    expect([...handlers.keys()].sort()).toEqual(['acp:instance-state', 'acp:permission-request', 'acp:transcript'])
  })

  it('routes acp:permission-request events into the per-instance usePermissions store', async () => {
    await startSessionStream()

    emit('acp:permission-request', {
      agent_id: 'a',
      session_id: 's-a',
      instance_id: 'A',
      request_id: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'echo hi',
      options: [{ option_id: 'allow', name: 'Allow', kind: 'y' }]
    })

    const pending = usePermissions('A').pending.value
    expect(pending).toHaveLength(1)
    expect(pending[0]?.requestId).toBe('req-1')
    expect(pending[0]?.tool).toBe('bash')
  })

  it('synthesizes distinct keys when the Rust emit omits request_id so concurrent prompts do not collide', async () => {
    await startSessionStream()

    emit('acp:permission-request', {
      agent_id: 'a',
      session_id: 's-a',
      instance_id: 'A',
      options: [{ option_id: 'allow', name: 'Allow', kind: 'y' }]
    })
    emit('acp:permission-request', {
      agent_id: 'a',
      session_id: 's-a',
      instance_id: 'A',
      options: [{ option_id: 'deny', name: 'Deny', kind: 'n' }]
    })

    const pending = usePermissions('A').pending.value
    expect(pending).toHaveLength(2)
    expect(new Set(pending.map((p) => p.requestId)).size).toBe(2)
    expect(pending.every((p) => p.tool === 'permission')).toBe(true)
  })

  it('routes acp:transcript events to the per-instance transcript store', async () => {
    await startSessionStream()

    emit('acp:transcript', {
      agent_id: 'a',
      session_id: 's-a',
      instance_id: 'A',
      update: { sessionUpdate: 'user_message_chunk', content: { type: 'text', text: 'hi' } }
    })
    emit('acp:transcript', {
      agent_id: 'a',
      session_id: 's-b',
      instance_id: 'B',
      update: { sessionUpdate: 'user_message_chunk', content: { type: 'text', text: 'yo' } }
    })

    const a = useTranscript('A').turns.value
    const b = useTranscript('B').turns.value
    expect(a.map((t) => t.text)).toEqual(['hi'])
    expect(b.map((t) => t.text)).toEqual(['yo'])
  })

  it('routes thought / plan / tool_call / terminal chunks to their respective stores', async () => {
    await startSessionStream()

    emit('acp:transcript', {
      agent_id: 'a',
      session_id: 's-a',
      instance_id: 'A',
      update: { sessionUpdate: 'agent_thought_chunk', content: { text: 'planning' } }
    })
    emit('acp:transcript', {
      agent_id: 'a',
      session_id: 's-a',
      instance_id: 'A',
      update: { sessionUpdate: 'plan', entries: [{ content: 'step-1' }] }
    })
    emit('acp:transcript', {
      agent_id: 'a',
      session_id: 's-a',
      instance_id: 'A',
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

    emit('acp:instance-state', { agent_id: 'a', instance_id: 'A', state: InstanceState.Starting })
    expect(id.value).toBeUndefined()

    emit('acp:instance-state', { agent_id: 'a', instance_id: 'A', state: InstanceState.Running })
    expect(id.value).toBe('A')

    emit('acp:instance-state', { agent_id: 'a', instance_id: 'B', state: InstanceState.Running })
    expect(id.value).toBe('A')
  })

  it('keeps routing stdout on tool_call_update chunks that omit `kind` after the initial tool_call', async () => {
    await startSessionStream()

    emit('acp:transcript', {
      agent_id: 'a',
      session_id: 's-a',
      instance_id: 'A',
      update: {
        sessionUpdate: 'tool_call',
        toolCallId: 'tc-bash',
        title: 'bash',
        kind: 'bash',
        rawInput: { command: 'tail -f log' },
        content: [{ type: 'text', text: 'line 1\n' }]
      }
    })
    emit('acp:transcript', {
      agent_id: 'a',
      session_id: 's-a',
      instance_id: 'A',
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
})
