import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { openTurnIdFor } from './use-turns'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import type { Attachment } from '@ipc'
import { paragraphSeparator } from '@lib/markdown'

export enum TurnRole {
  User = 'user',
  Agent = 'agent'
}

export interface ContentBlock {
  [k: string]: unknown
  type?: string
  text?: string
}

interface Turn {
  id: string
  sessionId: string
  /// Active ACP turn id at receive time. Only `Agent` turns can carry
  /// it (the user's chunk lands before any `TurnStarted` for the
  /// reply); `User` turns are always `undefined` here.
  turnId?: string
  createdAt: number
  updatedAt: number
}

export interface UserTurn extends Turn {
  role: TurnRole.User
  text: string
  /// Skill / image / resource attachments the user submitted alongside
  /// the text. Empty array when none. The collapsable in `Overlay.vue`
  /// reads this so the captain can re-inspect what context they fed
  /// into the turn.
  attachments: Attachment[]
}

export interface AgentTurn extends Turn {
  role: TurnRole.Agent
  text: string
  /// Agent-emitted attachments — image / audio / embedded resource /
  /// resource_link content blocks the agent streamed alongside (or
  /// instead of) text. Mirrors the user-side `attachments` field;
  /// the same `Attachments` chat component renders both. Empty
  /// array when the agent didn't attach anything.
  attachments: Attachment[]
  /// Most-recent vendor-emitted `messageId` on this turn's stream.
  /// Tracked so the chunk-fold logic can detect content-block
  /// boundaries and inject a markdown paragraph break (`\n\n`) when
  /// the next chunk's id changes — without it, vendors that switch
  /// ids mid-turn (Claude / Codex emit fresh ids per content block)
  /// concat blocks with no separator and markdown renders one
  /// run-on paragraph instead of two.
  lastChunkMessageId?: string
}

export type ChatTurnItem = UserTurn | AgentTurn

export interface TranscriptState {
  turns: ChatTurnItem[]
  /// Per-session id of the currently-open agent turn. While set, every
  /// `agent_message_chunk` folds into that turn — regardless of
  /// vendor-emitted `messageId` churn or stream items (thought / plan /
  /// tool-call) landing in sibling stores between chunks. Cleared on
  /// `TurnStarted` so the next turn opens a fresh block. Mirrors
  /// `openThoughtBySession` in use-stream.ts; without it the captain's
  /// reply renders as a stack of micro-cards instead of one flow.
  openAgentBySession: Map<string, string>
}

const states = reactive(new Map<InstanceId, TranscriptState>())

function slotFor(id: InstanceId): TranscriptState {
  let slot = states.get(id)

  if (!slot) {
    slot = { turns: [], openAgentBySession: new Map() }
    states.set(id, slot)
  }

  return slot
}

/// Close the per-session open-agent tracker — the next agent chunk
/// will open a fresh turn. Called from the session-stream demuxer on
/// `TurnStarted` (the canonical turn boundary), mirroring `closeTurn`
/// over in use-stream.ts.
export function closeTranscriptTurn(id: InstanceId, sessionId: string): void {
  const slot = states.get(id)

  if (!slot) {
    return
  }
  slot.openAgentBySession.delete(sessionId)
}

interface ChunkUpdate {
  sessionUpdate: string
  content?: ContentBlock
  messageId?: string
  /// Optional attachments — only meaningful on the first chunk of a
  /// user turn (`UserPrompt`). Merged onto the matching `UserTurn`
  /// when the chunk lands.
  attachments?: Attachment[]
}

function extractText(content?: ContentBlock): string {
  if (!content || typeof content.text !== 'string') {
    return ''
  }

  return content.text
}

function roleFor(sessionUpdate: string): TurnRole | undefined {
  switch (sessionUpdate) {
    case 'user_message_chunk':
      return TurnRole.User

    case 'agent_message_chunk':
      return TurnRole.Agent

    default:
      return undefined
  }
}

/// Append a chunk onto an open agent turn (or open a fresh one).
/// Extracted from `pushTranscriptChunk` to keep its cyclomatic
/// complexity under the project lint ceiling. The boundary heuristic
/// lives here too: a new vendor messageId on the open turn is the
/// signal that a content block ended, and markdown needs `\n\n`
/// between blocks or the captain reads two paragraphs as one.
function foldAgentChunk(
  instanceId: InstanceId,
  slot: TranscriptState,
  sessionId: string,
  seq: number,
  text: string,
  messageId: string | undefined,
  attachments: Attachment[]
): void {
  const openId = slot.openAgentBySession.get(sessionId)
  const target = openId !== undefined ? slot.turns.find((t): t is AgentTurn => t.role === TurnRole.Agent && t.sessionId === sessionId && t.id === openId) : undefined

  if (target) {
    const newBlock = messageId !== undefined && target.lastChunkMessageId !== undefined && target.lastChunkMessageId !== messageId

    if (newBlock) {
      target.text += paragraphSeparator(target.text, text)
    }
    target.text += text
    target.updatedAt = seq

    if (messageId !== undefined) {
      target.lastChunkMessageId = messageId
    }

    if (attachments.length > 0) {
      target.attachments = [...target.attachments, ...attachments]
    }

    return
  }
  const agentId = messageId ?? `agent-${sessionId}-${slot.turns.length}`

  slot.turns.push({
    role: TurnRole.Agent,
    id: agentId,
    sessionId,
    turnId: openTurnIdFor(instanceId, sessionId),
    createdAt: seq,
    updatedAt: seq,
    text,
    attachments,
    lastChunkMessageId: messageId
  })
  slot.openAgentBySession.set(sessionId, agentId)
}

// ── Internal store-mutation surface ───────────────────────────────
// Sibling-store wire-listener inputs. CLAUDE.md "Two-tier composables".

/**
 * Appends a chunk to the instance's transcript, merging consecutive
 * chunks that share `messageId` (or the same role with no explicit
 * id) into the same turn.
 */
export function pushTranscriptChunk(id: InstanceId, sessionId: string, raw: ChunkUpdate): void {
  const role = roleFor(raw.sessionUpdate)

  if (!role) {
    return
  }
  const text = extractText(raw.content)
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const hasExplicitId = typeof raw.messageId === 'string'

  // Agent chunks fold onto the open agent turn by id lookup, not by
  // "is the last item the agent turn?". Vendors interleave tool calls,
  // thoughts, and plans between agent_message_chunks — and some swap
  // the wire-side messageId between content blocks within a single
  // turn — both of which broke the old "last + role match" merge and
  // left the captain reading the reply as multiple cards.
  if (role === TurnRole.Agent) {
    foldAgentChunk(id, slot, sessionId, seq, text, raw.messageId, raw.attachments ?? [])

    return
  }

  // User chunks: keep the contiguous-last merge. User prompts arrive
  // as single shots in practice and there's no per-turn "open user
  // block" concept — the next user prompt is a fresh turn boundary.
  const last = slot.turns[slot.turns.length - 1]

  if (last && last.role === role && last.sessionId === sessionId && (hasExplicitId ? last.id === raw.messageId : true)) {
    last.text += text
    last.updatedAt = seq

    if (raw.attachments && raw.attachments.length > 0) {
      last.attachments = [...last.attachments, ...raw.attachments]
    }

    return
  }
  const userId = hasExplicitId ? (raw.messageId as string) : `user-${sessionId}-${slot.turns.length}`

  slot.turns.push({
    role: TurnRole.User,
    id: userId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    createdAt: seq,
    updatedAt: seq,
    text,
    attachments: raw.attachments ?? []
  })
}

/**
 * Resets an instance's transcript — used by `session_load` flows
 * once they need to clear-and-replay. Not wired yet.
 */
export function resetTranscript(id: InstanceId): void {
  states.delete(id)
}

/**
 * Remove every turn for a given ACP `turnId` from the instance's
 * transcript. Used by the cancel-turn affordance: when the user
 * cancels a turn, the cancelled user prompt + any partial agent
 * response stay in history by default; this lets the user opt to
 * delete them entirely so the chat reads cleanly.
 *
 * Removes both `User` (which carries the prompt) and `Agent` turns
 * tagged with the same `turnId`. The cancel toast pairs `pushTurnEnded`
 * with this so the surrounding state (open turn, pending tools,
 * permissions) tears down too.
 */
export function deleteTurnByTurnId(id: InstanceId, turnId: string): number {
  const slot = states.get(id)

  if (!slot) {
    return 0
  }
  const before = slot.turns.length

  slot.turns = slot.turns.filter((t) => t.turnId !== turnId)

  return before - slot.turns.length
}

export function useTranscript(instanceId?: InstanceId): { turns: ComputedRef<ChatTurnItem[]> } {
  const { id: activeId } = useActiveInstance()
  const turns = computed<ChatTurnItem[]>(() => {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return []
    }

    return states.get(resolved)?.turns ?? []
  })

  return { turns }
}
