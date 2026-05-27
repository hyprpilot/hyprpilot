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

import { StreamItemKind, type StreamItem } from './use-stream'
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
function projectEntry(it: SeqTranscriptItem, ctx: ProjectionContext): TimelineEntry | null {
  const { seq, item } = it

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
          id: it.messageId ?? `thought-${seq}`,
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

    case TranscriptItemKind.Compaction:
      return {
        kind: 'stream',
        createdAt: seq,
        item: {
          id: `compaction-${seq}`,
          kind: StreamItemKind.Compaction,
          sessionId: ctx.sessionId,
          createdAt: seq,
          updatedAt: seq,
          text: item.text,
          auto: item.auto,
          overflow: item.overflow,
          tailStartId: item.tailStartId
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
      // Live permission requests are owned by `usePermissions()`; the
      // chat cache doesn't render them while live. The SNAPSHOT path
      // (this branch fires when a snapshot RPC returns a still-pending
      // permission) gets rendered as a stream entry so the captain sees
      // an indicator where the prompt landed in history. nvim's renderer
      // forwards snapshot permission rows through the live row path too.
      return {
        kind: 'stream',
        createdAt: seq,
        item: {
          id: `perm-${seq}`,
          kind: StreamItemKind.Plan,
          sessionId: ctx.sessionId,
          createdAt: seq,
          updatedAt: seq,
          entries: [
            {
              content: `permission requested: ${item.tool}`,
              priority: 'medium',
              status: 'in_progress'
            }
          ]
        }
      } as TimelineStream
    case TranscriptItemKind.Unknown:
      // Daemon-version drift — flag in dev tools so the wire mismatch
      // is visible. Returning null still hides the row from the
      // viewport; the warning lets a captain debugging "missing
      // messages" see exactly which variant the daemon shipped.
      // eslint-disable-next-line no-console
      console.warn('snapshot-timeline: Unknown transcript item dropped', { seq, wireKind: item.wireKind })

      return null
  }

  // Forward-compat: TS exhaustiveness covers the closed set; the
  // explicit return guards against future variants.
  return null
}

interface ProjectedItem {
  seq: number
  turnId?: string
  entry: TimelineEntry
}

type SnapshotOverlayStreamItem = Extract<
  StreamItem,
  {
    kind: StreamItemKind.ModeChange | StreamItemKind.ModelChange | StreamItemKind.ConfigOptionChange | StreamItemKind.SystemPromptInjected
  }
>

export function isSnapshotOverlayStreamItem(item: StreamItem): item is SnapshotOverlayStreamItem {
  return (
    item.kind === StreamItemKind.ModeChange ||
    item.kind === StreamItemKind.ModelChange ||
    item.kind === StreamItemKind.ConfigOptionChange ||
    item.kind === StreamItemKind.SystemPromptInjected
  )
}

function overlaySeq(item: SnapshotOverlayStreamItem, snapshotItems: readonly SeqTranscriptItem[], ordinal: number): number {
  const offset = (ordinal + 1) / 1000

  if (item.kind === StreamItemKind.SystemPromptInjected) {
    return (snapshotItems[0]?.seq ?? 0) - 1 + offset
  }

  if (item.turnId !== undefined) {
    const lastTurnSeq = snapshotItems.reduce<number | undefined>((latest, snapshotItem) => {
      if (snapshotItem.turnId !== item.turnId) {
        return latest
      }

      return latest === undefined ? snapshotItem.seq : Math.max(latest, snapshotItem.seq)
    }, undefined)

    if (lastTurnSeq !== undefined) {
      return lastTurnSeq + offset
    }
  }

  return (snapshotItems[snapshotItems.length - 1]?.seq ?? 0) + 1 + ordinal
}

function projectOverlayStream(item: SnapshotOverlayStreamItem, seq: number): ProjectedItem {
  return {
    seq,
    turnId: item.turnId,
    entry: {
      kind: 'stream',
      createdAt: seq,
      item: {
        ...item,
        createdAt: seq,
        updatedAt: Math.max(item.updatedAt, seq)
      }
    } as TimelineStream
  }
}

function projectSnapshotItems(items: SeqTranscriptItem[], ctx: ProjectionContext): ProjectedItem[] {
  const projected: ProjectedItem[] = []

  for (const it of items) {
    const entry = projectEntry(it, ctx)

    if (!entry) {
      continue
    }

    if (tryMergeIntoExisting(projected, entry, it)) {
      continue
    }
    projected.push({
      seq: it.seq,
      turnId: it.turnId,
      entry
    })
  }

  return projected
}

function appendOverlayStreamItems(projected: ProjectedItem[], items: SeqTranscriptItem[], overlays: readonly StreamItem[]): void {
  for (const [idx, item] of overlays.filter(isSnapshotOverlayStreamItem).entries()) {
    projected.push(projectOverlayStream(item, overlaySeq(item, items, idx)))
  }
}

/**
 * Walk `projected` from the tail backwards looking for an item whose
 * `turnId` matches `turnId` AND that `predicate` accepts. Returns the
 * first match (most recent). Returns `undefined` when an item with a
 * DIFFERENT turnId (or `undefined`) appears before the match — that
 * gap is an explicit logical break in the captain's reading order and
 * the new entry must not fold across it.
 *
 * This is the load-bearing rule for agent-text + thought folding:
 * interleaved items WITHIN the same turn (same turnId) are transparent,
 * but a foreign-turnId or unkeyed item is a hard boundary.
 */
function findFoldTargetWithinTurn(projected: ProjectedItem[], turnId: string, predicate: (p: ProjectedItem) => boolean): ProjectedItem | undefined {
  for (let i = projected.length - 1; i >= 0; i -= 1) {
    const p = projected[i]

    if (!p) {
      continue
    }

    // Foreign-turnId (or unkeyed) item is a hard logical break — the
    // captain read past it, so the new entry must not fold into
    // anything older. Bail before the predicate check.
    if (p.turnId !== turnId) {
      return undefined
    }

    if (predicate(p)) {
      return p
    }
  }

  return undefined
}

/**
 * Try to merge a turn entry into an existing one. Agent-text chunks
 * fold across interleaving items (thoughts, tool calls, plans) within
 * the same turn — matches the live path's `openAgentBySession`
 * semantics. The wire ships one SeqTranscriptItem per streamed chunk;
 * without folding by turnId, a turn with N text chunks and an
 * interleaved thought renders as N+ separate Body bubbles in the same
 * block. User entries stand alone (each prompt is a turn boundary);
 * AgentAttachment carries its own row and stays unmerged. An unkeyed
 * (undefined-turnId) item between two same-turnId chunks splits the
 * fold — handled by `findFoldTargetWithinTurn`.
 */
function tryMergeTurn(projected: ProjectedItem[], entry: TimelineEntry & { kind: 'turn' }, it: SeqTranscriptItem): boolean {
  if (entry.turn.role === TurnRole.Agent && it.turnId !== undefined && entry.turn.attachments.length === 0) {
    const target = findFoldTargetWithinTurn(projected, it.turnId, (p) => p.entry.kind === 'turn' && p.entry.turn.role === TurnRole.Agent && p.entry.turn.attachments.length === 0)

    if (target && target.entry.kind === 'turn') {
      // Verbatim concat — the daemon bakes any content-block
      // boundary prefix onto each `AgentText` chunk before emission.
      // Snapshot replay must append exactly what the live path saw.
      target.entry.turn.text += entry.turn.text
      target.entry.turn.updatedAt = it.seq

      return true
    }
  }

  const prev = projected[projected.length - 1]

  if (prev && prev.entry.kind === 'turn' && prev.entry.turn.role === entry.turn.role && prev.turnId === it.turnId) {
    prev.entry.turn.text += entry.turn.text
    prev.entry.turn.updatedAt = it.seq

    if (entry.turn.attachments.length > 0) {
      prev.entry.turn.attachments = [...prev.entry.turn.attachments, ...entry.turn.attachments]
    }

    return true
  }

  return false
}

/**
 * Try to merge a Thought stream entry into an existing one. Thoughts
 * fold by turnId for the same reason as agent-text — interleaved tool
 * calls between thought chunks must not split the thought stream. An
 * unkeyed item between same-turnId chunks splits the fold.
 */
function tryMergeThought(projected: ProjectedItem[], entry: TimelineStream, it: SeqTranscriptItem): boolean {
  if (entry.item.kind !== StreamItemKind.Thought) {
    return false
  }

  if (it.turnId !== undefined) {
    let target: ProjectedItem | undefined

    for (let i = projected.length - 1; i >= 0; i -= 1) {
      const p = projected[i]

      if (!p || p.turnId !== it.turnId) {
        break
      }

      if (p.entry.kind === 'tool') {
        break
      }

      if (p.entry.kind === 'stream' && p.entry.item.kind === StreamItemKind.Thought) {
        const sameMessage = it.messageId !== undefined ? p.entry.item.id === it.messageId : p.entry.item.id.startsWith('thought-')

        if (sameMessage) {
          target = p
        }

        break
      }
    }

    if (target && target.entry.kind === 'stream' && target.entry.item.kind === StreamItemKind.Thought) {
      // Verbatim concat — daemon-side chunk text is already final.
      target.entry.item.text += entry.item.text
      target.entry.item.updatedAt = it.seq

      return true
    }

    return false
  }

  // Unkeyed thought (turnId === undefined): walk backwards through
  // contiguous unkeyed items and fold into the first Thought we hit.
  // Stops on any keyed item — that's a hard turn boundary, the
  // captain has read past it. Mirrors how nvim accumulates thoughts
  // into `state.active_thought_block` until a turn header lands.
  // Without this branch, two unkeyed thought chunks separated by an
  // unkeyed tool call would land as separate cards instead of one.
  for (let i = projected.length - 1; i >= 0; i -= 1) {
    const p = projected[i]

    if (!p) {
      continue
    }

    if (p.turnId !== undefined) {
      return false
    }

    if (p.entry.kind === 'stream' && p.entry.item.kind === StreamItemKind.Thought) {
      p.entry.item.text += entry.item.text
      p.entry.item.updatedAt = it.seq

      return true
    }
  }

  return false
}

/**
 * Try to overwrite an existing Plan entry for the same turnId. Plans
 * arrive as full snapshots; later updates within the same turn
 * supersede earlier ones. Without this each plan ACP update lands as
 * its own card and the captain reads the same plan N times.
 */
function tryMergePlan(projected: ProjectedItem[], entry: TimelineStream, it: SeqTranscriptItem): boolean {
  const existing = projected.find((p) => p.entry.kind === 'stream' && p.entry.item.kind === StreamItemKind.Plan && p.turnId === it.turnId)

  if (existing && existing.entry.kind === 'stream' && existing.entry.item.kind === StreamItemKind.Plan && entry.item.kind === StreamItemKind.Plan) {
    existing.entry.item.entries = entry.item.entries
    existing.entry.item.updatedAt = it.seq

    return true
  }

  return false
}

/**
 * Try to merge a tool-call entry by `toolCallId`.
 */
function tryMergeTool(projected: ProjectedItem[], entry: TimelineTool, it: SeqTranscriptItem): boolean {
  const existing = projected.find((p) => p.entry.kind === 'tool' && p.entry.call.toolCallId === entry.call.toolCallId)

  if (existing && existing.entry.kind === 'tool') {
    mergeToolCall(existing.entry.call, entry.call, it.seq)

    return true
  }

  return false
}

/**
 * Try to merge `entry` into an existing item in `projected`. Returns
 * true when the merge happened (caller skips the push). The wire ships
 * one TranscriptItem per streamed chunk; the rendered view is one
 * message per logical reply — same merge the live router does at push
 * time, replayed here for the snapshot path.
 */
function tryMergeIntoExisting(projected: ProjectedItem[], entry: TimelineEntry, it: SeqTranscriptItem): boolean {
  if (entry.kind === 'turn') {
    return tryMergeTurn(projected, entry, it)
  }

  if (entry.kind === 'stream' && entry.item.kind === StreamItemKind.Thought) {
    return tryMergeThought(projected, entry, it)
  }

  if (entry.kind === 'stream' && entry.item.kind === StreamItemKind.Plan) {
    return tryMergePlan(projected, entry, it)
  }

  if (entry.kind === 'tool') {
    return tryMergeTool(projected, entry, it)
  }

  return false
}

/// Ranks tool-call states by progress so a stale `ToolCall` arriving
/// AFTER a `ToolCallUpdate` (out-of-order in the snapshot stream, or
/// the patcher pushed a `ToolCallUpdate` as an orphan before the
/// matching `ToolCall` landed) can't downgrade `completed`/`failed`
/// back to `pending`/`running`. The rule: only accept the incoming
/// status when its rank is >= the existing one.
const TOOL_STATE_RANK: Record<string, number> = {
  pending: 0,
  in_progress: 1,
  running: 1,
  completed: 2,
  failed: 2,
  cancelled: 2
}

function mergeToolCall(target: WireToolCall, incoming: WireToolCall, seq: number): void {
  if (incoming.title !== undefined) {
    target.title = incoming.title
  }

  if (incoming.status !== undefined) {
    const existingRank = TOOL_STATE_RANK[(target.status ?? '').toLowerCase()] ?? 0
    const incomingRank = TOOL_STATE_RANK[incoming.status.toLowerCase()] ?? 0

    if (incomingRank >= existingRank) {
      target.status = incoming.status
    }
  }

  if (incoming.kind !== undefined) {
    target.kind = incoming.kind
  }

  if (incoming.content && incoming.content.length > 0) {
    target.content = [...(target.content ?? []), ...incoming.content]
  }

  if (incoming.rawInput !== undefined) {
    target.rawInput = incoming.rawInput
  }
  target.formatted = incoming.formatted

  if (incoming.completedAtMs !== undefined) {
    target.completedAtMs = incoming.completedAtMs
  }
  target.updatedAt = seq
}

/**
 * Convert oldest-first SeqTranscriptItem[] into TimelineBlocks.
 * Mirrors `useTimelineBlocks`'s grouping rules:
 *
 * - Items carrying a `turnId` group with consecutive items sharing
 *   the same id (assistant blocks anchored on the ACP turn). The
 *   user prompt that opened the turn lives there too — same turnId,
 *   but lands in its own user-role block per the live router's
 *   "user entry always solo" rule.
 * - Items without a `turnId` (synthetic / pre-turn agent activity,
 *   spontaneous updates) fall back to role-run grouping — every
 *   consecutive run of assistant entries lands in one block, every
 *   user entry in its own.
 */
export function timelineBlocksFromSnapshot(items: SeqTranscriptItem[], sessionId = 'snapshot', overlays: readonly StreamItem[] = []): TimelineBlock[] {
  const ctx: ProjectionContext = { sessionId }
  const projected = projectSnapshotItems(items, ctx)

  appendOverlayStreamItems(projected, items, overlays)
  projected.sort((a, b) => a.seq - b.seq || KIND_ORDER[a.entry.kind] - KIND_ORDER[b.entry.kind])

  const out: TimelineBlock[] = []
  let assistantRunIdx = 0

  for (const { entry, turnId } of projected) {
    const role = entry.kind === 'turn' ? (entry.turn.role === TurnRole.User ? Role.User : Role.Assistant) : Role.Assistant
    // User entries always sit in their own block — even when they
    // carry a turnId (the prompt that opened the turn). Mirrors the
    // live router's `solo:user:...` keying.
    const groupKey = role === Role.User ? `snapshot-user:${entry.createdAt}` : turnId !== undefined ? `turn:${turnId}` : `snapshot-assistant:${assistantRunIdx}`
    const last = out[out.length - 1]
    let block: TimelineBlock

    if (last && last.groupKey === groupKey) {
      block = last
    } else {
      // Bump the assistant run counter on every assistant→non-assistant
      // and non-assistant→assistant transition so role-run grouping
      // for `turnId === undefined` items doesn't collapse two
      // separate runs that were split by an interleaving user block.
      if (role === Role.Assistant && turnId === undefined) {
        if (last && (last.role !== Role.Assistant || last.turnId !== undefined)) {
          assistantRunIdx += 1
        }
      }
      const finalGroupKey = role === Role.User ? groupKey : turnId !== undefined ? `turn:${turnId}` : `snapshot-assistant:${assistantRunIdx}`

      block = {
        role,
        groupKey: finalGroupKey,
        turnId: role === Role.Assistant ? turnId : undefined,
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
      if (entry.call.kind?.type === 'think') {
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
