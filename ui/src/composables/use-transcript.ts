import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { useActiveInstance, type InstanceId } from './use-active-instance'
import { openTurnIdFor } from './use-turns'

export enum TurnRole {
  User = 'user',
  Agent = 'agent'
}

export interface ContentBlock {
  type?: string
  text?: string
  [k: string]: unknown
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
}

export interface AgentTurn extends Turn {
  role: TurnRole.Agent
  text: string
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
    text
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
