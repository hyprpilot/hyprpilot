import { describe, expect, it } from 'vitest'

import { timelineBlocksFromSnapshot } from './snapshot-timeline'
import { Role } from '@components'
import { TranscriptItemKind } from '@constants/wire/transcript'
import { PlanPriority, PlanStepStatus, type PlanStep } from '@interfaces/wire/transcript'
import type { SeqTranscriptItem } from '@ipc'

function userPrompt(seq: number, turnId: string | undefined, text: string): SeqTranscriptItem {
  return {
    seq,
    turnId,
    item: {
      kind: TranscriptItemKind.UserPrompt,
      text,
      attachments: []
    }
  }
}

function agentText(seq: number, turnId: string | undefined, text: string): SeqTranscriptItem {
  return {
    seq,
    turnId,
    item: { kind: TranscriptItemKind.AgentText, text }
  }
}

function agentThought(seq: number, turnId: string | undefined, text: string): SeqTranscriptItem {
  return {
    seq,
    turnId,
    item: { kind: TranscriptItemKind.AgentThought, text }
  }
}

describe('timelineBlocksFromSnapshot', () => {
  it('groups consecutive items by turnId and lays user prompts in their own block', () => {
    // user prompt (no turnId — emitted before TurnStarted) → agent t-1 chunks → agent t-2 chunks
    const items: SeqTranscriptItem[] = [userPrompt(1, undefined, 'hello'), agentText(2, 't-1', 'reply 1a'), agentThought(3, 't-1', 'thinking'), agentText(4, 't-2', 'reply 2a')]
    const blocks = timelineBlocksFromSnapshot(items)

    expect(blocks).toHaveLength(3)
    expect(blocks[0].role).toBe(Role.User)
    expect(blocks[0].turnId).toBeUndefined()
    expect(blocks[1].role).toBe(Role.Assistant)
    expect(blocks[1].turnId).toBe('t-1')
    expect(blocks[1].turnEntries).toHaveLength(1)
    expect(blocks[1].streamEntries).toHaveLength(1)
    expect(blocks[2].role).toBe(Role.Assistant)
    expect(blocks[2].turnId).toBe('t-2')
    expect(blocks[2].turnEntries).toHaveLength(1)
  })

  it('user prompts carrying a turnId still land in their own block (matches live router)', () => {
    const items: SeqTranscriptItem[] = [userPrompt(1, 't-1', 'submit'), agentText(2, 't-1', 'agent reply')]
    const blocks = timelineBlocksFromSnapshot(items)

    expect(blocks).toHaveLength(2)
    expect(blocks[0].role).toBe(Role.User)
    expect(blocks[1].role).toBe(Role.Assistant)
    expect(blocks[1].turnId).toBe('t-1')
  })

  it('falls back to role-run grouping for items without a turnId', () => {
    // Two agent items with no turnId in a row collapse into one
    // assistant block; a user item splits the run. Adjacent agent
    // chunks merge into one turnEntry — they're a single message
    // streamed in pieces, not two separate replies.
    const items: SeqTranscriptItem[] = [
      agentText(1, undefined, 'pre-turn agent 1'),
      agentText(2, undefined, 'pre-turn agent 2'),
      userPrompt(3, undefined, 'user'),
      agentText(4, undefined, 'post-user agent')
    ]
    const blocks = timelineBlocksFromSnapshot(items)

    expect(blocks).toHaveLength(3)
    expect(blocks[0].role).toBe(Role.Assistant)
    expect(blocks[0].turnEntries).toHaveLength(1)
    expect(blocks[0].turnEntries[0].turn.text).toBe('pre-turn agent 1pre-turn agent 2')
    expect(blocks[1].role).toBe(Role.User)
    expect(blocks[2].role).toBe(Role.Assistant)
    expect(blocks[2].turnEntries).toHaveLength(1)
  })

  it('merges streamed agent text chunks within a turn into one turnEntry', () => {
    // Captain's bug: each agent_message_chunk landed as its own row
    // because the snapshot ships one TranscriptItem per chunk and
    // the projector did not merge. Live path merges via messageId;
    // snapshot path merges by adjacency + turnId.
    const items: SeqTranscriptItem[] = [
      userPrompt(1, 't-1', 'hey'),
      agentText(2, 't-1', 'Hey! '),
      agentText(3, 't-1', 'Doing well — '),
      agentText(4, 't-1', 'what\'s on your mind?')
    ]
    const blocks = timelineBlocksFromSnapshot(items)

    expect(blocks).toHaveLength(2)
    expect(blocks[1].role).toBe(Role.Assistant)
    expect(blocks[1].turnEntries).toHaveLength(1)
    expect(blocks[1].turnEntries[0].turn.text).toBe('Hey! Doing well — what\'s on your mind?')
  })

  it('does not collapse two distinct turnIds even when adjacent', () => {
    const items: SeqTranscriptItem[] = [agentText(1, 't-1', 'a'), agentText(2, 't-2', 'b')]
    const blocks = timelineBlocksFromSnapshot(items)

    expect(blocks).toHaveLength(2)
    expect(blocks[0].turnId).toBe('t-1')
    expect(blocks[1].turnId).toBe('t-2')
  })

  it('does not collapse turnId blocks with adjacent un-keyed (undefined turnId) items', () => {
    const items: SeqTranscriptItem[] = [agentText(1, 't-1', 'a'), agentText(2, undefined, 'b'), agentText(3, 't-1', 'c')]
    const blocks = timelineBlocksFromSnapshot(items)

    // turn t-1 block, role-run block (no turnId), turn t-1 block again
    expect(blocks).toHaveLength(3)
    expect(blocks[0].turnId).toBe('t-1')
    expect(blocks[1].turnId).toBeUndefined()
    expect(blocks[2].turnId).toBe('t-1')
  })

  it('overwrites plan stream items within a turn instead of stacking them', () => {
    // Plans arrive as full snapshots — every ACP `plan` update carries
    // the complete current step list. The daemon mirror appends one
    // SeqTranscriptItem per update, so the snapshot of a turn that
    // received N plan updates ships N plan items. Without dedup the
    // captain reads the same plan N times stacked vertically.
    const planItem = (seq: number, turnId: string, steps: PlanStep[]): SeqTranscriptItem => ({
      seq,
      turnId,
      item: {
        kind: TranscriptItemKind.Plan,
        steps
      }
    })
    const items: SeqTranscriptItem[] = [
      userPrompt(1, 't-1', 'go'),
      planItem(2, 't-1', [
        {
          content: 'step 1',
          status: PlanStepStatus.Pending,
          priority: PlanPriority.High
        }
      ]),
      planItem(3, 't-1', [
        {
          content: 'step 1',
          status: PlanStepStatus.Completed,
          priority: PlanPriority.High
        },
        {
          content: 'step 2',
          status: PlanStepStatus.Pending,
          priority: PlanPriority.Medium
        }
      ]),
      planItem(4, 't-1', [
        {
          content: 'step 1',
          status: PlanStepStatus.Completed,
          priority: PlanPriority.High
        },
        {
          content: 'step 2',
          status: PlanStepStatus.Completed,
          priority: PlanPriority.Medium
        }
      ])
    ]
    const blocks = timelineBlocksFromSnapshot(items)
    const assistantBlock = blocks.find((b) => b.role === Role.Assistant)

    expect(assistantBlock?.streamEntries).toHaveLength(1)
    const planEntry = assistantBlock?.streamEntries[0]

    expect(planEntry?.item.kind).toBe('plan')

    if (planEntry?.item.kind === 'plan') {
      // Latest plan wins — both steps completed.
      expect(planEntry.item.entries).toHaveLength(2)
      expect(planEntry.item.entries[1].status).toBe(PlanStepStatus.Completed)
    }
  })
})
