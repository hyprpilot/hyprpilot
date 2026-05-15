import { beforeEach, describe, expect, it } from 'vitest'

import { closeTranscriptTurn, useActiveInstance, pushTranscriptChunk, resetTranscript, TurnRole, useTranscript } from '@composables'

beforeEach(() => {
  resetTranscript('A')
  resetTranscript('B')
  useActiveInstance().id.value = undefined
})

function chunk(sessionUpdate: string, text: string, messageId?: string) {
  return {
    sessionUpdate,
    content: { type: 'text', text },
    messageId
  }
}

describe('useTranscript', () => {
  it('routes events to the correct per-instance slice', () => {
    pushTranscriptChunk('A', 's-a', chunk('user_message_chunk', 'hi from A'))
    pushTranscriptChunk('B', 's-b', chunk('user_message_chunk', 'hi from B'))

    const a = useTranscript('A').turns
    const b = useTranscript('B').turns

    expect(a.value).toHaveLength(1)
    expect(a.value[0]?.text).toBe('hi from A')
    expect(a.value[0]?.role).toBe(TurnRole.User)

    expect(b.value).toHaveLength(1)
    expect(b.value[0]?.text).toBe('hi from B')
  })

  it('isolates instance slices: A never sees B turns and vice versa', () => {
    pushTranscriptChunk('A', 's-a', chunk('user_message_chunk', 'alpha'))
    pushTranscriptChunk('B', 's-b', chunk('user_message_chunk', 'beta'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'gamma'))

    const a = useTranscript('A').turns.value
    const b = useTranscript('B').turns.value

    expect(a.map((t) => t.text)).toEqual(['alpha', 'gamma'])
    expect(b.map((t) => t.text)).toEqual(['beta'])
  })

  it('merges consecutive same-role chunks into one turn', () => {
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'hel', 'm-1'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'lo', 'm-1'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', ' world'))

    const turns = useTranscript('A').turns.value

    expect(turns).toHaveLength(1)
    expect(turns[0]?.text).toBe('hello world')
  })

  it('resolves through useActiveInstance when no id is passed', () => {
    useActiveInstance().set('A')
    pushTranscriptChunk('A', 's-a', chunk('user_message_chunk', 'active'))
    pushTranscriptChunk('B', 's-b', chunk('user_message_chunk', 'background'))

    const implicit = useTranscript().turns.value

    expect(implicit.map((t) => t.text)).toEqual(['active'])
  })

  it('returns empty array when instance has no state yet', () => {
    expect(useTranscript('nonexistent').turns.value).toEqual([])
  })

  it('folds agent chunks with different messageIds into the open agent turn', () => {
    // Vendors swap messageIds mid-turn (Claude / Codex emit fresh ids
    // per content block). Without the open-agent tracker, every id
    // change spawned a fresh card; the captain reads one logical
    // reply as a stack of micro-cards. Pin the contract: across
    // distinct ids in the same turn the chunks fold.
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'first ', 'm-1'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'block ', 'm-2'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'block', 'm-3'))

    const turns = useTranscript('A').turns.value

    expect(turns).toHaveLength(1)
    expect(turns[0]?.text).toBe('first block block')
  })

  it('folds agent chunks after a user prompt lands between them', () => {
    // User prompts land in the same store as agent turns. The previous
    // merge logic relied on `last` being the agent turn; a user prompt
    // (or any non-agent item arriving after the agent's first chunk
    // — though in practice the demuxer routes those elsewhere) would
    // break the merge. Belt-and-braces: even if a user prompt lands
    // mid-agent without a TurnStarted reset, the open tracker holds.
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'agent-part-1', 'm-1'))
    pushTranscriptChunk('A', 's-a', chunk('user_message_chunk', 'interruption'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', '-part-2', 'm-1'))

    const turns = useTranscript('A').turns.value
    const agentTurns = turns.filter((t) => t.role === TurnRole.Agent)

    expect(agentTurns).toHaveLength(1)
    expect(agentTurns[0]?.text).toBe('agent-part-1-part-2')
  })

  it('closeTranscriptTurn opens a fresh agent block for the next chunk', () => {
    // Pin the turn-boundary contract: `closeTranscriptTurn` is the
    // demuxer's TurnStarted hook. After it runs, the next agent chunk
    // must NOT fold onto the previous turn — that's a separate reply.
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'turn one', 'm-1'))
    closeTranscriptTurn('A', 's-a')
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'turn two', 'm-2'))

    const turns = useTranscript('A').turns.value.filter((t) => t.role === TurnRole.Agent)

    expect(turns).toHaveLength(2)
    expect(turns[0]?.text).toBe('turn one')
    expect(turns[1]?.text).toBe('turn two')
  })
})
