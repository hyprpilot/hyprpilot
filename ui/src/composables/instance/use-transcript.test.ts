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

  it('folds agent chunks with different messageIds into the open agent turn with markdown paragraph breaks', () => {
    // Vendors swap messageIds mid-turn (Claude / Codex emit fresh ids
    // per content block). Without the open-agent tracker, every id
    // change spawned a fresh card; the captain read one logical reply
    // as a stack of micro-cards. Now distinct ids in the same turn
    // fold into one card AND get `\n\n` between each block so markdown
    // renders them as separate paragraphs (the captain's screenshot
    // bug — three blocks were running together as one paragraph).
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'first', 'm-1'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'block', 'm-2'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'block', 'm-3'))

    const turns = useTranscript('A').turns.value

    expect(turns).toHaveLength(1)
    expect(turns[0]?.text).toBe('first\n\nblock\n\nblock')
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

  it('inserts a paragraph break between merged agent chunks with different messageIds', () => {
    // Captain's screenshot bug: a single agent turn folded from two
    // vendor content blocks (distinct messageIds) used to render as
    // one markdown paragraph because the join was bare text. Pin the
    // contract: when the second chunk's messageId differs from what
    // we last folded, prepend `\n\n` so markdown sees a paragraph
    // boundary.
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'First paragraph.', 'm-1'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'Second paragraph.', 'm-2'))

    const agentTurn = useTranscript('A').turns.value.find((t) => t.role === TurnRole.Agent)

    expect(agentTurn?.text).toBe('First paragraph.\n\nSecond paragraph.')
  })

  it('does not duplicate paragraph break when an existing one is already present', () => {
    // The vendor sometimes ends its own content block with `\n\n`
    // before swapping messageId on the next one. Don't stack another
    // separator on top — that would render as three blank lines.
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'First paragraph.\n\n', 'm-1'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'Second paragraph.', 'm-2'))

    const agentTurn = useTranscript('A').turns.value.find((t) => t.role === TurnRole.Agent)

    expect(agentTurn?.text).toBe('First paragraph.\n\nSecond paragraph.')
  })

  it('does not insert a paragraph break when consecutive chunks share a messageId', () => {
    // Multi-chunk content-block stream within one paragraph (vendor
    // streams a long sentence token-by-token). Same messageId = same
    // content block = no separator.
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'hello ', 'm-1'))
    pushTranscriptChunk('A', 's-a', chunk('agent_message_chunk', 'world', 'm-1'))

    const agentTurn = useTranscript('A').turns.value.find((t) => t.role === TurnRole.Agent)

    expect(agentTurn?.text).toBe('hello world')
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
