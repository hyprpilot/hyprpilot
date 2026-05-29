import { describe, expect, it } from 'vitest'

import { timelineBlocksFromSnapshot } from './snapshot-timeline'
import { StreamItemKind, type StreamItem } from './use-stream'
import { Role } from '@components'
import { ChangeAdvertisementType, TranscriptItemKind } from '@constants/wire/transcript'
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

function agentThought(seq: number, turnId: string | undefined, text: string, messageId?: string): SeqTranscriptItem {
  return {
    seq,
    turnId,
    messageId,
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
    // daemon bakes any content-boundary prefix onto chunk text
    // BEFORE emission, so the projector just concatenates — no
    // client-side separator logic.
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

  it('preserves daemon-baked content-boundary prefixes when folding chunks (verbatim concat)', () => {
    // The daemon prefixes explicit content-block id switches with
    // `\n\n`. The projector's job is to faithfully concatenate
    // the chunks the daemon ships — no further mutation.
    const items: SeqTranscriptItem[] = [userPrompt(1, 't-1', 'go'), agentText(2, 't-1', 'Paragraph one.\n'), agentText(3, 't-1', '\nParagraph two.')]
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

  it('splits thought chunks across an interleaved tool call within the same turn', () => {
    // OpenAI/opencode can emit distinct reasoning parts around tool
    // calls while reusing the message id. A tool boundary means the
    // second reasoning chunk belongs in a new thinking block.
    const toolCall = (seq: number, turnId: string, id: string): SeqTranscriptItem => ({
      seq,
      turnId,
      item: {
        kind: TranscriptItemKind.ToolCall,
        id,
        title: 'edit',
        state: 'completed',
        toolKind: { type: 'edit' },
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
    expect(assistant?.streamEntries).toHaveLength(2)
    const [first, second] = assistant?.streamEntries ?? []

    if (first?.item.kind === 'thought') {
      expect(first.item.text).toBe('first half — ')
    }

    if (second?.item.kind === 'thought') {
      expect(second.item.text).toBe('second half.')
    }
  })

  it('uses thought messageId to keep distinct thought blocks separate during snapshot replay', () => {
    const items: SeqTranscriptItem[] = [
      userPrompt(1, 't-1', 'go'),
      agentThought(2, 't-1', 'first half ', 'thought-a'),
      agentThought(3, 't-1', 'continues', 'thought-a'),
      agentThought(4, 't-1', 'second thought', 'thought-b')
    ]
    const blocks = timelineBlocksFromSnapshot(items)
    const assistant = blocks.find((b) => b.role === Role.Assistant)

    expect(assistant?.streamEntries).toHaveLength(2)
    const [first, second] = assistant?.streamEntries ?? []

    if (first?.item.kind === 'thought') {
      expect(first.item.text).toBe('first half continues')
      expect(first.item.id).toBe('thought-a')
    }

    if (second?.item.kind === 'thought') {
      expect(second.item.text).toBe('second thought')
      expect(second.item.id).toBe('thought-b')
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
        steps,
        stats: { done: 0, total: steps.length }
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

  it('overwrites goal stream items within a turn instead of stacking them', () => {
    const goalItem = (seq: number, turnId: string, status: string, objective: string): SeqTranscriptItem => ({
      seq,
      turnId,
      item: {
        kind: TranscriptItemKind.Goal,
        status,
        objective
      }
    })
    const items: SeqTranscriptItem[] = [userPrompt(1, 't-1', 'go'), goalItem(2, 't-1', 'active', 'ship goal parsing'), goalItem(3, 't-1', 'blocked', 'needs input')]
    const blocks = timelineBlocksFromSnapshot(items)
    const assistantBlock = blocks.find((b) => b.role === Role.Assistant)

    expect(assistantBlock?.streamEntries).toHaveLength(1)
    const goalEntry = assistantBlock?.streamEntries[0]

    expect(goalEntry?.item.kind).toBe(StreamItemKind.Goal)

    if (goalEntry?.item.kind === StreamItemKind.Goal) {
      expect(goalEntry.item.status).toBe('blocked')
      expect(goalEntry.item.objective).toBe('needs input')
    }
  })

  it('projects compaction transcript items into stream blocks', () => {
    const items: SeqTranscriptItem[] = [
      userPrompt(1, 't-1', 'go'),
      {
        seq: 2,
        turnId: 't-1',
        item: {
          kind: TranscriptItemKind.Compaction,
          text: 'summary',
          auto: true,
          overflow: true,
          tailStartId: 'm-1'
        }
      }
    ]
    const blocks = timelineBlocksFromSnapshot(items)
    const assistantBlock = blocks.find((b) => b.role === Role.Assistant)
    const compaction = assistantBlock?.streamEntries[0]?.item

    expect(compaction?.kind).toBe(StreamItemKind.Compaction)

    if (compaction?.kind === StreamItemKind.Compaction) {
      expect(compaction.text).toBe('summary')
      expect(compaction.auto).toBe(true)
      expect(compaction.overflow).toBe(true)
      expect(compaction.tailStartId).toBe('m-1')
    }
  })

  it('projects durable change advertisement transcript items into stream blocks', () => {
    const items: SeqTranscriptItem[] = [
      userPrompt(1, 't-1', 'switch'),
      {
        seq: 2,
        turnId: 't-1',
        item: {
          kind: TranscriptItemKind.ChangeAdvertisement,
          type: ChangeAdvertisementType.Mode,
          value: 'build',
          name: 'Build',
          prevValue: 'plan',
          prevName: 'Plan'
        }
      },
      {
        seq: 3,
        turnId: 't-1',
        item: {
          kind: TranscriptItemKind.ChangeAdvertisement,
          type: ChangeAdvertisementType.Model,
          value: 'gpt-5.5',
          name: 'GPT-5.5',
          prevValue: 'gpt-5',
          prevName: 'GPT-5'
        }
      },
      {
        seq: 4,
        turnId: 't-1',
        item: {
          kind: TranscriptItemKind.ChangeAdvertisement,
          type: ChangeAdvertisementType.ConfigOption,
          categoryId: 'effort',
          value: 'high',
          name: 'High',
          prevValue: 'medium',
          prevName: 'Medium'
        }
      }
    ]

    const blocks = timelineBlocksFromSnapshot(items)
    const assistantBlock = blocks.find((b) => b.role === Role.Assistant)

    expect(assistantBlock?.streamEntries.map((entry) => entry.item.kind)).toEqual([StreamItemKind.ModeChange, StreamItemKind.ModelChange, StreamItemKind.ConfigOptionChange])
    expect(assistantBlock?.streamEntries[0]?.item).toMatchObject({
      kind: StreamItemKind.ModeChange,
      modeId: 'build',
      prevModeId: 'plan'
    })
    expect(assistantBlock?.streamEntries[1]?.item).toMatchObject({
      kind: StreamItemKind.ModelChange,
      modelId: 'gpt-5.5',
      prevModelId: 'gpt-5'
    })
    expect(assistantBlock?.streamEntries[2]?.item).toMatchObject({
      kind: StreamItemKind.ConfigOptionChange,
      categoryId: 'effort',
      value: 'high',
      prevValue: 'medium'
    })
  })

  it('overlays live change banners onto snapshot-rendered turn blocks', () => {
    const items: SeqTranscriptItem[] = [userPrompt(1, 't-1', 'switch'), agentText(2, 't-1', 'done')]
    const overlays: StreamItem[] = [
      {
        kind: StreamItemKind.ModeChange,
        id: 'mode-s-a-0',
        sessionId: 's-a',
        turnId: 't-1',
        createdAt: 1,
        updatedAt: 1,
        modeId: 'default',
        name: 'Default',
        prevModeId: 'plan',
        prevName: 'Plan'
      },
      {
        kind: StreamItemKind.ModelChange,
        id: 'model-s-a-1',
        sessionId: 's-a',
        turnId: 't-1',
        createdAt: 2,
        updatedAt: 2,
        modelId: 'gpt-5.5',
        name: 'GPT-5.5',
        prevModelId: 'gpt-5',
        prevName: 'GPT-5'
      },
      {
        kind: StreamItemKind.ConfigOptionChange,
        id: 'cfg-effort-s-a-2',
        sessionId: 's-a',
        turnId: 't-1',
        createdAt: 3,
        updatedAt: 3,
        categoryId: 'effort',
        value: 'high',
        name: 'High',
        prevValue: 'medium',
        prevName: 'Medium'
      }
    ]

    const blocks = timelineBlocksFromSnapshot(items, 'snapshot', overlays)
    const assistantBlock = blocks.find((b) => b.role === Role.Assistant)

    expect(assistantBlock?.streamEntries.map((entry) => entry.item.kind)).toEqual([StreamItemKind.ModeChange, StreamItemKind.ModelChange, StreamItemKind.ConfigOptionChange])
    expect(assistantBlock?.streamEntries[0]?.item).toMatchObject({
      kind: StreamItemKind.ModeChange,
      modeId: 'default',
      prevModeId: 'plan'
    })
  })

  it('places pre-turn change banners before the first snapshot turn', () => {
    const items: SeqTranscriptItem[] = [userPrompt(10, undefined, 'what mode are you in?'), agentText(11, 't-1', 'I am in build mode.')]
    const overlays: StreamItem[] = [
      {
        kind: StreamItemKind.ModelChange,
        id: 'model-s-a-0',
        sessionId: 's-a',
        turnId: undefined,
        createdAt: 1,
        updatedAt: 1,
        modelId: 'gpt-5',
        name: 'GPT-5',
        prevModelId: 'gpt-5.3-codex-spark',
        prevName: 'GPT-5.3 Codex Spark'
      },
      {
        kind: StreamItemKind.ModeChange,
        id: 'mode-s-a-1',
        sessionId: 's-a',
        turnId: undefined,
        createdAt: 2,
        updatedAt: 2,
        modeId: 'build',
        name: 'Build',
        prevModeId: 'plan',
        prevName: 'Plan'
      },
      {
        kind: StreamItemKind.ConfigOptionChange,
        id: 'cfg-effort-s-a-2',
        sessionId: 's-a',
        turnId: undefined,
        createdAt: 3,
        updatedAt: 3,
        categoryId: 'effort',
        value: 'xhigh',
        name: 'XHigh',
        prevValue: 'none',
        prevName: 'None'
      }
    ]

    const blocks = timelineBlocksFromSnapshot(items, 'snapshot', overlays)

    expect(blocks[0].role).toBe(Role.Assistant)
    expect(blocks[0].turnId).toBeUndefined()
    expect(blocks[0].streamEntries.map((entry) => entry.item.kind)).toEqual([StreamItemKind.ModelChange, StreamItemKind.ModeChange, StreamItemKind.ConfigOptionChange])
    expect(blocks[1].role).toBe(Role.User)
    expect(blocks[1].turnEntries[0]?.turn.text).toBe('what mode are you in?')
    expect(blocks[2].role).toBe(Role.Assistant)
    expect(blocks[2].turnEntries[0]?.turn.text).toBe('I am in build mode.')
  })

  it('places system prompt overlay banners before the first snapshot turn', () => {
    const overlays: StreamItem[] = [
      {
        kind: StreamItemKind.SystemPromptInjected,
        id: 'system-prompt-A-0',
        sessionId: '',
        createdAt: 1,
        updatedAt: 1,
        files: ['/tmp/base.md']
      }
    ]

    const blocks = timelineBlocksFromSnapshot([userPrompt(10, undefined, 'hello')], 'snapshot', overlays)

    expect(blocks[0].streamEntries[0]?.item).toMatchObject({
      kind: StreamItemKind.SystemPromptInjected,
      files: ['/tmp/base.md']
    })
    expect(blocks[1].role).toBe(Role.User)
  })
})
