/**
 * Build TimelineBlock[] from a flat SeqTranscriptItem[] (the daemon
 * snapshot shape). Mirrors `useTimelineBlocks` (which reads off the
 * accumulator stores) for the snapshot-driven chat body (Phase C1).
 *
 * Why a parallel implementation rather than re-using `useTimelineBlocks`:
 *
 * - The accumulator stores (`use-transcript`, `use-stream`, `use-tools`)
 *   stay populated by the live event router for now — they still drive
 *   the composer's phase derivation, the queue, and the `useTurns`
 *   counter (CLAUDE.md "Two-tier composables" surface). Phase C3
 *   deletes the interim caps but keeps the stores.
 * - The body view (`<ChatBody>`) needs the *truth* from the daemon
 *   snapshot, not the accumulator caps. The infinite-query pages are
 *   that truth. Re-using `useTimelineBlocks` would re-derive blocks
 *   from the accumulator (capped at 500 turns) instead of from the
 *   snapshot — defeating the whole virtualized-viewport design.
 *
 * One pure function: `(items: SeqTranscriptItem[]) → TimelineBlock[]`.
 * No reactive state; the caller wraps it in a `computed`.
 */

import { StreamItemKind } from './use-stream'
import { type TimelineBlock, type TimelineEntry, type TimelineStream, type TimelineTool, type TimelineTurn } from './use-timeline-blocks'
import { TurnRole, type ChatTurnItem } from './use-transcript'
import { Role } from '@components'
import type { WireToolCall } from '@interfaces/ui'
import { type SeqTranscriptItem, TranscriptItemKind, type TranscriptItem } from '@ipc'

const KIND_ORDER = {
  turn: 0,
  stream: 1,
  tool: 2
} as const

interface ProjectionContext {
  /// Synthetic session id for snapshot items — the daemon's
  /// transcript items don't all carry one explicitly. We share one
  /// per instance so consecutive turn merges line up.
  sessionId: string
}

function projectTurn(
  item:
    | Extract<TranscriptItem, { kind: TranscriptItemKind.UserPrompt }>
    | Extract<TranscriptItem, { kind: TranscriptItemKind.UserText }>
    | Extract<TranscriptItem, { kind: TranscriptItemKind.AgentText }>
    | Extract<TranscriptItem, { kind: TranscriptItemKind.AgentAttachment }>,
  seq: number,
  ctx: ProjectionContext
): ChatTurnItem {
  if (item.kind === TranscriptItemKind.UserPrompt || item.kind === TranscriptItemKind.UserText) {
    return {
      role: TurnRole.User,
      id: `user-${seq}`,
      sessionId: ctx.sessionId,
      createdAt: seq,
      updatedAt: seq,
      text: item.kind === TranscriptItemKind.UserPrompt ? item.text : item.text,
      attachments: item.kind === TranscriptItemKind.UserPrompt ? item.attachments : []
    }
  }

  if (item.kind === TranscriptItemKind.AgentAttachment) {
    return {
      role: TurnRole.Agent,
      id: `agent-${seq}`,
      sessionId: ctx.sessionId,
      createdAt: seq,
      updatedAt: seq,
      text: '',
      attachments: [
        {
          slug: item.slug,
          path: item.path,
          body: item.body,
          title: item.title,
          data: item.data,
          mime: item.mime
        }
      ]
    }
  }

  return {
    role: TurnRole.Agent,
    id: `agent-${seq}`,
    sessionId: ctx.sessionId,
    createdAt: seq,
    updatedAt: seq,
    text: item.text,
    attachments: []
  }
}

function projectToolCall(
  item: Extract<TranscriptItem, { kind: TranscriptItemKind.ToolCall | TranscriptItemKind.ToolCallUpdate }>,
  seq: number,
  ctx: ProjectionContext
): WireToolCall {
  return {
    id: item.id,
    agentId: '',
    sessionId: ctx.sessionId,
    turnId: undefined,
    toolCallId: item.id,
    title: item.title,
    status: item.state,
    kind: item.toolKind,
    content: (item.content ?? []).map((c) => {
      if (c.kind === 'text') {
        return { type: 'text', text: c.text }
      }

      if (c.kind === 'file') {
        return {
          type: 'file',
          path: c.path,
          snippet: c.snippet
        }
      }

      return { type: 'json', value: c.value }
    }),
    rawInput: item.rawInput,
    locations: undefined,
    formatted: item.formatted,
    startedAtMs: item.startedAtMs,
    completedAtMs: item.completedAtMs,
    createdAt: seq,
    updatedAt: seq
  }
}

/**
 * Project one snapshot item onto a TimelineEntry. `null` for shapes
 * the body doesn't render (PermissionRequest, Unknown, Plan handled
 * separately).
 */
function projectEntry(seq: number, item: TranscriptItem, ctx: ProjectionContext): TimelineEntry | null {
  switch (item.kind) {
    case TranscriptItemKind.UserPrompt:
    case TranscriptItemKind.UserText:
    case TranscriptItemKind.AgentText:
    case TranscriptItemKind.AgentAttachment:
      return {
        kind: 'turn',
        createdAt: seq,
        turn: projectTurn(item, seq, ctx)
      }

    case TranscriptItemKind.AgentThought:
      return {
        kind: 'stream',
        createdAt: seq,
        item: {
          id: `thought-${seq}`,
          kind: StreamItemKind.Thought,
          sessionId: ctx.sessionId,
          createdAt: seq,
          updatedAt: seq,
          text: item.text,
          startedAtMs: 0
        }
      } as TimelineStream

    case TranscriptItemKind.Plan:
      return {
        kind: 'stream',
        createdAt: seq,
        item: {
          id: `plan-${seq}`,
          kind: StreamItemKind.Plan,
          sessionId: ctx.sessionId,
          createdAt: seq,
          updatedAt: seq,
          entries: item.steps.map((step) => ({
            content: step.content,
            priority: step.priority,
            status: step.status
          }))
        }
      } as TimelineStream

    case TranscriptItemKind.ToolCall:
    case TranscriptItemKind.ToolCallUpdate:
      return {
        kind: 'tool',
        createdAt: seq,
        call: projectToolCall(item, seq, ctx)
      } as TimelineTool

    case TranscriptItemKind.PermissionRequest:
    case TranscriptItemKind.Unknown:
      return null
  }

  // Forward-compat: TS exhaustiveness covers the closed set; the
  // explicit return guards against future variants.
  return null
}

/**
 * Convert oldest-first SeqTranscriptItem[] into TimelineBlocks.
 * Mirrors `useTimelineBlocks`'s grouping rules: assistant entries
 * sharing a snapshot run collapse into one block; user entries
 * always sit in their own block.
 */
export function timelineBlocksFromSnapshot(items: SeqTranscriptItem[], sessionId = 'snapshot'): TimelineBlock[] {
  const ctx: ProjectionContext = { sessionId }
  const entries: TimelineEntry[] = []

  for (const it of items) {
    const projected = projectEntry(it.seq, it.item, ctx)

    if (projected) {
      entries.push(projected)
    }
  }

  entries.sort((a, b) => a.createdAt - b.createdAt || KIND_ORDER[a.kind] - KIND_ORDER[b.kind])

  const out: TimelineBlock[] = []
  // The snapshot doesn't carry ACP turn ids inline (those ride on
  // sibling `turn-started` / `turn-ended` events the mirror records
  // separately). Group by role-run instead — every consecutive run
  // of assistant entries lands in one block, every user entry in
  // its own. Mirrors what the live router achieves once turn ids
  // arrive on the events.
  let assistantRunIdx = 0

  for (const entry of entries) {
    const role = entry.kind === 'turn' ? (entry.turn.role === TurnRole.User ? Role.User : Role.Assistant) : Role.Assistant
    const groupKey = role === Role.Assistant ? `snapshot-assistant:${assistantRunIdx}` : `snapshot-user:${entry.createdAt}`
    const last = out[out.length - 1]
    let block: TimelineBlock

    if (last && last.groupKey === groupKey) {
      block = last
    } else {
      // Bump the assistant run counter when transitioning OUT of an
      // assistant block — next assistant content opens a new block.
      if (last && last.role === Role.Assistant && role !== Role.Assistant) {
        assistantRunIdx += 1
      } else if (last && last.role !== Role.Assistant && role === Role.Assistant) {
        // Transitioning back into an assistant role bumps the run
        // counter so the new block doesn't accidentally collapse
        // with a prior assistant block whose entries came earlier.
        assistantRunIdx += 1
      }
      block = {
        role,
        groupKey: role === Role.Assistant ? `snapshot-assistant:${assistantRunIdx}` : groupKey,
        turnId: undefined,
        startedAt: entry.createdAt,
        streamEntries: [],
        toolCalls: [],
        thoughts: [],
        turnEntries: []
      }
      out.push(block)
    }

    if (entry.kind === 'stream') {
      block.streamEntries.push(entry)
    } else if (entry.kind === 'tool') {
      if (entry.call.kind?.toLowerCase() === 'think') {
        block.thoughts.push(entry as TimelineTool)
      } else {
        block.toolCalls.push(entry as TimelineTool)
      }
    } else {
      block.turnEntries.push(entry as TimelineTurn)
    }
  }

  return out
}
