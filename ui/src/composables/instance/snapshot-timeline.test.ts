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

  it('merges streamed agent text chunks within a turn into one turnEntry (verbatim concat)', () => {
    // Each `agent_message_chunk` lands as a separate
    // SeqTranscriptItem; the projector folds them by turnId. The
    // daemon bakes any markdown-paragraph-lift prefix onto the
    // chunk text BEFORE emission (see `adapters/acp/paragraph.rs`),
    // so the projector just concatenates — no client-side
    // separator logic.
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

  it('preserves daemon-baked paragraph-lift prefixes when folding chunks (verbatim concat)', () => {
    // The daemon's `TurnState::note_agent_text` prepends a `\n` to
    // a chunk when the prior accumulated text ended with a single
    // `\n`, lifting the boundary to `\n\n` for proper markdown
    // paragraph rendering. The projector's job is to faithfully
    // concatenate the chunks the daemon ships — no further
    // mutation. This test pins that contract.
    const items: SeqTranscriptItem[] = [
      userPrompt(1, 't-1', 'go'),
      agentText(2, 't-1', 'Paragraph one.\n'),
      agentText(3, 't-1', '\nParagraph two.')
    ]
    const blocks = timelineBlocksFromSnapshot(items)
    const assistant = blocks.find((b) => b.role === Role.Assistant)

    expect(assistant?.turnEntries).toHaveLength(1)
    expect(assistant?.turnEntries[0].turn.text).toBe('Paragraph one.\n\nParagraph two.')
  })

  it('folds agent text chunks across an interleaved thought within the same turn', () => {
    // Regression: vendors (Claude / Codex / OpenCode) emit
    // agent_thought between agent_message_chunks of the same turn,
    // and the snapshot ships each as a separate SeqTranscriptItem.
    // The adjacency-only merge produced multiple Body bubbles in one
    // block; the captain read one logical reply as a stack of cards.
    const items: SeqTranscriptItem[] = [
      userPrompt(1, 't-1', 'hey'),
      agentText(2, 't-1', 'Paragraph one. '),
      agentThought(3, 't-1', 'thinking…'),
      agentText(4, 't-1', 'Paragraph two.')
    ]
    const blocks = timelineBlocksFromSnapshot(items)
    const assistant = blocks.find((b) => b.role === Role.Assistant)

    expect(assistant).toBeDefined()
    expect(assistant?.turnEntries).toHaveLength(1)
    expect(assistant?.turnEntries[0].turn.text).toBe('Paragraph one. Paragraph two.')
    expect(assistant?.streamEntries).toHaveLength(1)
  })

  it('folds thought chunks across an interleaved tool call within the same turn', () => {
    // Same shape as the agent-text fold, applied to the Thought
    // stream — a tool call between thought chunks must not split
    // the thinking block.
    const toolCall = (seq: number, turnId: string, id: string): SeqTranscriptItem => ({
      seq,
      turnId,
      item: {
        kind: TranscriptItemKind.ToolCall,
        id,
        title: 'edit',
        state: 'completed',
        toolKind: 'edit',
        content: [],
        rawInput: {},
        formatted: {
          title: 'edit',
          stats: [],
          description: '',
          output: '',
          fields: [],
          iconKey: 'edit'
        }
      } as unknown as SeqTranscriptItem['item']
    })

    const items: SeqTranscriptItem[] = [userPrompt(1, 't-1', 'go'), agentThought(2, 't-1', 'first half — '), toolCall(3, 't-1', 'tc-1'), agentThought(4, 't-1', 'second half.')]
    const blocks = timelineBlocksFromSnapshot(items)
    const assistant = blocks.find((b) => b.role === Role.Assistant)

    expect(assistant).toBeDefined()
    expect(assistant?.streamEntries).toHaveLength(1)
    const thought = assistant?.streamEntries[0]

    expect(thought?.item.kind).toBe('thought')

    if (thought?.item.kind === 'thought') {
      expect(thought.item.text).toBe('first half — second half.')
    }
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
