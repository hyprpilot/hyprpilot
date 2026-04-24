import { beforeEach, describe, expect, it, vi } from 'vitest'

import { InstanceState } from '@ipc'

import { Phase } from '@components'

import { useActiveInstance } from '@composables/use-active-instance'
import { resetPermissions, pushPermissionRequest } from '@composables/use-permissions'
import { __resetAllPhaseSignals, pushInstanceState, usePhase } from '@composables/use-phase'
import { resetTools, pushToolCall } from '@composables/use-tools'
import { pushTranscriptChunk, resetTranscript } from '@composables/use-transcript'

vi.mock('@ipc', async () => {
  const actual = await vi.importActual<typeof import('@ipc')>('@ipc')
  return {
    ...actual,
    invoke: vi.fn(),
    listen: vi.fn()
  }
})

beforeEach(() => {
  __resetAllPhaseSignals()
  resetTranscript('A')
  resetTranscript('B')
  resetTools('A')
  resetTools('B')
  resetPermissions('A')
  resetPermissions('B')
  useActiveInstance().id.value = undefined
})

describe('usePhase', () => {
  it('returns idle when no active instance is set', () => {
    const { phase } = usePhase()
    expect(phase.value).toBe(Phase.Idle)
  })

  it('returns working when instance is running but has no agent turns and no tools or perms', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)

    const { phase } = usePhase()
    expect(phase.value).toBe(Phase.Working)
  })

  it('returns streaming when instance is running and an agent turn has arrived', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTranscriptChunk('A', 's-a', {
      sessionUpdate: 'agent_message_chunk',
      content: { type: 'text', text: 'hello' }
    })

    const { phase } = usePhase()
    expect(phase.value).toBe(Phase.Streaming)
  })

  it('returns pending when a tool call is running (beats streaming)', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTranscriptChunk('A', 's-a', {
      sessionUpdate: 'agent_message_chunk',
      content: { type: 'text', text: 'hi' }
    })
    pushToolCall('A', 's-a', {
      sessionUpdate: 'tool_call',
      toolCallId: 'tc-1',
      title: 'bash',
      kind: 'bash',
      status: 'running'
    })

    const { phase } = usePhase()
    expect(phase.value).toBe(Phase.Pending)
  })

  it('returns awaiting when there is a pending permission prompt (beats pending)', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushToolCall('A', 's-a', {
      sessionUpdate: 'tool_call',
      toolCallId: 'tc-1',
      title: 'bash',
      status: 'running'
    })
    pushPermissionRequest('A', 's-a', {
      request_id: 'req-1',
      tool: 'bash',
      kind: 'bash',
      args: 'echo hi',
      options: [{ option_id: 'allow', name: 'Allow', kind: 'y' }]
    })

    const { phase } = usePhase()
    expect(phase.value).toBe(Phase.Awaiting)
  })

  it('returns idle when instance state is ended', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushInstanceState('A', InstanceState.Ended)

    const { phase } = usePhase()
    expect(phase.value).toBe(Phase.Idle)
  })

  it('isolates instances: pushing signals for A does not affect B', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTranscriptChunk('A', 's-a', {
      sessionUpdate: 'agent_message_chunk',
      content: { type: 'text', text: 'from A' }
    })

    const phaseA = usePhase('A')
    const phaseB = usePhase('B')

    expect(phaseA.phase.value).toBe(Phase.Streaming)
    expect(phaseB.phase.value).toBe(Phase.Idle)
  })

  it('resolves the explicit instanceId arg over the active id', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushInstanceState('B', InstanceState.Running)
    pushTranscriptChunk('B', 's-b', {
      sessionUpdate: 'agent_message_chunk',
      content: { type: 'text', text: 'from B' }
    })

    const { phase } = usePhase('B')
    expect(phase.value).toBe(Phase.Streaming)
  })
})
