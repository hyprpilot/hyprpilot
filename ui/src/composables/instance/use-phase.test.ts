import { beforeEach, describe, expect, it, vi } from 'vitest'

import { Phase } from '@components'
import {
  useActiveInstance,
  resetPermissions,
  pushPermissionRequest,
  __resetAllPhaseSignals,
  pushInstanceState,
  usePhase,
  resetTools,
  pushToolCall,
  pushTranscriptChunk,
  resetTranscript,
  pushTurnEnded,
  pushTurnStarted,
  resetTurns
} from '@composables'
import { InstanceState } from '@ipc'

const fmt = {
  title: 'bash',
  stats: [],
  fields: []
}

vi.mock('@ipc', async() => {
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
  resetTurns('A')
  resetTurns('B')
  useActiveInstance().id.value = undefined
})

describe('usePhase', () => {
  it('returns idle when no active instance is set', () => {
    const { phase } = usePhase()

    expect(phase.value).toBe(Phase.Idle)
  })

  it('returns working when instance is running with an open turn but no agent chunks yet', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTurnStarted('A', {
      turnId: 't-1', sessionId: 's-a', startedAtMs: 0
    })

    const { phase } = usePhase()

    expect(phase.value).toBe(Phase.Working)
  })

  it('returns streaming when instance is running, a turn is open, and agent chunks have arrived', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTurnStarted('A', {
      turnId: 't-1', sessionId: 's-a', startedAtMs: 0
    })
    pushTranscriptChunk('A', 's-a', {
      sessionUpdate: 'agent_message_chunk',
      content: { type: 'text', text: 'hello' }
    })

    const { phase } = usePhase()

    expect(phase.value).toBe(Phase.Streaming)
  })

  it('returns idle in-between turns even when prior agent turns exist (queue-stuck regression)', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTurnStarted('A', {
      turnId: 't-1', sessionId: 's-a', startedAtMs: 0
    })
    pushTranscriptChunk('A', 's-a', {
      sessionUpdate: 'agent_message_chunk',
      content: { type: 'text', text: 'hello' }
    })
    pushTurnEnded('A', {
      turnId: 't-1',
      sessionId: 's-a',
      stopReason: 'end_turn', endedAtMs: 0
    })

    const { phase } = usePhase()

    expect(phase.value).toBe(Phase.Idle)
  })

  it('returns pending when a tool call is running (beats streaming)', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTurnStarted('A', {
      turnId: 't-1', sessionId: 's-a', startedAtMs: 0
    })
    pushTranscriptChunk('A', 's-a', {
      sessionUpdate: 'agent_message_chunk',
      content: { type: 'text', text: 'hi' }
    })
    pushToolCall('A', 'agent-A', 's-a', {
      sessionUpdate: 'tool_call',
      toolCallId: 'tc-1',
      title: 'bash',
      kind: 'bash',
      status: 'running',
      formatted: fmt, startedAtMs: 0
    })

    const { phase } = usePhase()

    expect(phase.value).toBe(Phase.Pending)
  })

  it('returns awaiting when there is a pending permission prompt (beats pending)', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTurnStarted('A', {
      turnId: 't-1', sessionId: 's-a', startedAtMs: 0
    })
    pushToolCall('A', 'agent-A', 's-a', {
      sessionUpdate: 'tool_call',
      toolCallId: 'tc-1',
      title: 'bash',
      status: 'running',
      formatted: fmt, startedAtMs: 0
    })
    pushPermissionRequest('A', 's-a', {
      agentId: 'agent-A',
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
      formatted: fmt
    })

    const { phase } = usePhase()

    expect(phase.value).toBe(Phase.Awaiting)
  })

  it('returns idle when instance state is ended', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTurnStarted('A', {
      turnId: 't-1', sessionId: 's-a', startedAtMs: 0
    })
    pushInstanceState('A', InstanceState.Ended)

    const { phase } = usePhase()

    expect(phase.value).toBe(Phase.Idle)
  })

  it('isolates instances: pushing signals for A does not affect B', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTurnStarted('A', {
      turnId: 't-1', sessionId: 's-a', startedAtMs: 0
    })
    pushTranscriptChunk('A', 's-a', {
      sessionUpdate: 'agent_message_chunk',
      content: { type: 'text', text: 'from A' }
    })

    const phaseA = usePhase('A')
    const phaseB = usePhase('B')

    expect(phaseA.phase.value).toBe(Phase.Streaming)
    expect(phaseB.phase.value).toBe(Phase.Idle)
  })

  it('returns idle when replayed tool calls have non-terminal status but no turn is open (session-restore regression)', () => {
    // Session restore replays historical `tool_call` updates with the
    // suspended-time status (`in_progress`, `pending`). Replays don't
    // emit `acp:turn-started`, so `openTurnId` stays undefined.
    // Phase must NOT register these as live work or the composer
    // locks forever on the resumed session.
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushToolCall('A', 'agent-A', 's-a', {
      sessionUpdate: 'tool_call',
      toolCallId: 'tc-replayed',
      title: 'bash',
      kind: 'bash',
      status: 'in_progress',
      formatted: fmt, startedAtMs: 0
    })

    const { phase } = usePhase()

    expect(phase.value).toBe(Phase.Idle)
  })

  it('drops foreign-session orphans on TurnStarted (queue-stuck regression)', () => {
    // Captain reports prompts queueing on a "fresh" instance even
    // when phase reads idle. Hypothesis: a prior session's turn
    // never received its TurnEnded (broadcast dropped, daemon
    // panic mid-turn, restart sequence fired Ended → Running too
    // tightly), so `openBySession` carried an orphan keyed by the
    // dead session. The new session's TurnStarted then ended fine,
    // but `openTurnId` (which reads `values().next()`) hit the
    // orphan, pinned phase != Idle, and routed the next prompt
    // into the queue. pushTurnStarted now drops orphans for
    // foreign sessions before setting its own entry.
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTurnStarted('A', {
      turnId: 't-old', sessionId: 's-orphan', startedAtMs: 0
    })
    // Simulate the orphan: a new TurnStarted lands on a different
    // session WITHOUT the prior session's TurnEnded clearing.
    pushTurnStarted('A', {
      turnId: 't-new', sessionId: 's-fresh', startedAtMs: 0
    })
    // The new turn ends cleanly.
    pushTurnEnded('A', {
      turnId: 't-new', sessionId: 's-fresh', stopReason: 'end_turn', endedAtMs: 0
    })

    const { phase } = usePhase()

    // Without the foreign-session drop the phase would be Working
    // (orphan turn 't-old' still open under 's-orphan').
    expect(phase.value).toBe(Phase.Idle)
  })

  it('resolves the explicit instanceId arg over the active id', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushInstanceState('B', InstanceState.Running)
    pushTurnStarted('B', {
      turnId: 't-1', sessionId: 's-b', startedAtMs: 0
    })
    pushTranscriptChunk('B', 's-b', {
      sessionUpdate: 'agent_message_chunk',
      content: { type: 'text', text: 'from B' }
    })

    const { phase } = usePhase('B')

    expect(phase.value).toBe(Phase.Streaming)
  })
})
