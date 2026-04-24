import { beforeEach, describe, expect, it } from 'vitest'

import { useActiveInstance } from '@composables/use-active-instance'
import {
  pushTranscriptChunk,
  resetTranscript,
  TurnRole,
  useTranscript
} from '@composables/use-transcript'

beforeEach(() => {
  resetTranscript('A')
  resetTranscript('B')
  useActiveInstance().id.value = undefined
})

function chunk(sessionUpdate: string, text: string, messageId?: string) {
  return { sessionUpdate, content: { type: 'text', text }, messageId }
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
})
