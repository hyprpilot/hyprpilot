import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { openTurnIdFor } from './use-turns'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import type { Attachment } from '@ipc'

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
}

export type ChatTurnItem = UserTurn | AgentTurn

export interface TranscriptState {
  turns: ChatTurnItem[]
}

const states = reactive(new Map<InstanceId, TranscriptState>())

function slotFor(id: InstanceId): TranscriptState {
  let slot = states.get(id)

  if (!slot) {
    slot = { turns: [] }
    states.set(id, slot)
  }

  return slot
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
  const last = slot.turns[slot.turns.length - 1]

  if (last && last.role === role && last.sessionId === sessionId && (hasExplicitId ? last.id === raw.messageId : true)) {
    last.text += text
    last.updatedAt = seq

    // Attachments append to the existing turn (instead of replacing)
    // so an agent that streams multiple non-text content blocks lands
    // every one in the same turn alongside the text. User-side keeps
    // the same shape — user attachments only ride on the first chunk
    // anyway.
    if (raw.attachments && raw.attachments.length > 0) {
      last.attachments = [...last.attachments, ...raw.attachments]
    }

    return
  }
  const messageId = hasExplicitId ? (raw.messageId as string) : `${role}-${sessionId}-${slot.turns.length}`
  const turn: ChatTurnItem = {
    role,
    id: messageId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    createdAt: seq,
    updatedAt: seq,
    text,
    attachments: raw.attachments ?? []
  }

  slot.turns.push(turn)
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
