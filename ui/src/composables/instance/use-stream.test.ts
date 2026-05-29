import { beforeEach, describe, expect, it } from 'vitest'

import { pushCompaction, pushGoal, pushPlan, pushThoughtChunk, resetStream, StreamItemKind, useStream } from '@composables'

beforeEach(() => {
  resetStream('A')
  resetStream('B')
})

describe('useStream', () => {
  it('isolates thought + plan streams between instances', () => {
    pushThoughtChunk('A', 's-a', { sessionUpdate: 'agent_thought_chunk', content: { text: 'thinking A' } })
    pushPlan('A', 's-a', { sessionUpdate: 'plan', entries: [{ content: 'step-a1' }] })
    pushThoughtChunk('B', 's-b', { sessionUpdate: 'agent_thought_chunk', content: { text: 'thinking B' } })

    const a = useStream('A').items.value
    const b = useStream('B').items.value

    expect(a).toHaveLength(2)
    expect(b).toHaveLength(1)
    expect(a[0]?.kind).toBe(StreamItemKind.Thought)
    expect(a[1]?.kind).toBe(StreamItemKind.Plan)
    expect(b[0]?.kind).toBe(StreamItemKind.Thought)
    expect(b[0]).not.toBe(a[0])
  })

  it('merges consecutive thought chunks with the same messageId', () => {
    pushThoughtChunk('A', 's-a', {
      sessionUpdate: 'agent_thought_chunk',
      content: { text: 'hel' },
      messageId: 't-1'
    })
    pushThoughtChunk('A', 's-a', {
      sessionUpdate: 'agent_thought_chunk',
      content: { text: 'lo' },
      messageId: 't-1'
    })

    const items = useStream('A').items.value

    expect(items).toHaveLength(1)
    expect(items[0]?.kind === StreamItemKind.Thought ? items[0].text : null).toBe('hello')
  })

  it('replaces entries on the open plan item across the same turn', () => {
    pushPlan('A', 's-a', { sessionUpdate: 'plan', entries: [{ content: 'e1' }] })
    pushPlan('A', 's-a', { sessionUpdate: 'plan', entries: [{ content: 'e2' }, { content: 'e3' }] })

    const items = useStream('A').items.value

    expect(items).toHaveLength(1)
    expect(items[0]?.kind === StreamItemKind.Plan ? items[0].entries.length : null).toBe(2)
  })

  it('replaces goal state on the open goal item across the same turn', () => {
    pushGoal('A', 's-a', { status: 'active', objective: 'ship it' })
    pushGoal('A', 's-a', { status: 'blocked', objective: 'needs input' })

    const items = useStream('A').items.value

    expect(items).toHaveLength(1)
    expect(items[0]?.kind).toBe(StreamItemKind.Goal)

    if (items[0]?.kind === StreamItemKind.Goal) {
      expect(items[0].status).toBe('blocked')
      expect(items[0].objective).toBe('needs input')
    }
  })

  it('pushes compaction stream items', () => {
    pushCompaction('A', 's-a', {
      text: 'summary',
      auto: true,
      overflow: true,
      tailStartId: 'm-1'
    })

    const items = useStream('A').items.value

    expect(items).toHaveLength(1)
    expect(items[0]?.kind).toBe(StreamItemKind.Compaction)

    if (items[0]?.kind === StreamItemKind.Compaction) {
      expect(items[0].text).toBe('summary')
      expect(items[0].auto).toBe(true)
      expect(items[0].overflow).toBe(true)
      expect(items[0].tailStartId).toBe('m-1')
    }
  })
})
