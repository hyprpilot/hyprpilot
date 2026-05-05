import { beforeEach, describe, expect, it } from 'vitest'

import { pushThoughtChunk, pushTurnStarted, resetStream, resetTurns, StreamItemKind, useStream, useTimelineBlocks } from '@composables'
import { useActiveInstance } from '@composables/chrome/use-active-instance'

beforeEach(() => {
  resetStream('A')
  resetTurns('A')
  useActiveInstance().set('A')
})

describe('thinking block rendering plumbing', () => {
  it('agent_thought_chunk arrives → stream item with text + turnId, surfaces in timeline block', () => {
    pushTurnStarted('A', {
      turnId: 't-live', sessionId: 's-a', startedAtMs: 1000
    })

    pushThoughtChunk('A', 's-a', {
      sessionUpdate: 'agent_thought_chunk',
      content: { text: 'I will analyze' }
    })

    const items = useStream('A').items.value

    expect(items).toHaveLength(1)
    const thought = items[0]

    expect(thought.kind).toBe(StreamItemKind.Thought)
    expect(thought.kind === StreamItemKind.Thought ? thought.text : '').toBe('I will analyze')
    expect(thought.turnId).toBe('t-live')

    const blocks = useTimelineBlocks().blocks.value

    expect(blocks).toHaveLength(1)
    expect(blocks[0].turnId).toBe('t-live')
    expect(blocks[0].streamEntries).toHaveLength(1)
    expect(blocks[0].streamEntries[0].item.kind).toBe(StreamItemKind.Thought)
  })

  it('thought arriving BEFORE TurnStarted goes to a solo block (no turnId)', () => {
    // Race case: agent_thought_chunk lands before acp:turn-started.
    // The thought has no turnId → solo block keyed off createdAt.
    pushThoughtChunk('A', 's-a', {
      sessionUpdate: 'agent_thought_chunk',
      content: { text: 'race thought' }
    })

    const items = useStream('A').items.value

    expect(items).toHaveLength(1)
    expect(items[0].turnId).toBeUndefined()

    const blocks = useTimelineBlocks().blocks.value

    expect(blocks).toHaveLength(1)
    expect(blocks[0].turnId).toBeUndefined()
    expect(blocks[0].streamEntries).toHaveLength(1)
  })
})
