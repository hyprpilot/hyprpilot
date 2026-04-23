import { computed, type Ref } from 'vue'

import type { TranscriptEvent } from './useAdapter'

/**
 * Local view of an ACP `SessionUpdate`. Upstream carries 8+
 * `#[non_exhaustive]` variants; we collapse them into the five the
 * chat shell renders and drop the rest as `Unknown`. Matches the
 * `Chat*` primitive tree under `components/chat/` (`ChatUserBody`,
 * `ChatAssistantBody`, `ChatStreamCard`, `ChatToolChips`).
 */
export enum MessageKind {
  User = 'user',
  AgentMessage = 'agent_message',
  AgentThought = 'agent_thought',
  AgentToolCall = 'agent_tool_call',
  AgentPlan = 'agent_plan',
  Unknown = 'unknown'
}

export interface ContentBlock {
  type?: string
  text?: string
  [k: string]: unknown
}

interface BaseMessage {
  id: string
  sessionId: string
  updatedAt: number
}

export interface UserChatMessage extends BaseMessage {
  kind: MessageKind.User
  text: string
}

export interface AgentChatMessage extends BaseMessage {
  kind: MessageKind.AgentMessage
  text: string
}

export interface AgentThoughtMessage extends BaseMessage {
  kind: MessageKind.AgentThought
  text: string
}

export interface ToolCallLocation {
  path?: string
  line?: number
}

export interface ToolCallSnapshot {
  toolCallId: string
  title?: string
  status?: string
  kind?: string
  content: ContentBlock[]
  rawInput?: Record<string, unknown>
  locations?: ToolCallLocation[]
}

export interface AgentToolCallMessage extends BaseMessage {
  kind: MessageKind.AgentToolCall
  call: ToolCallSnapshot
}

export interface PlanEntry {
  content?: string
  status?: string
  priority?: string
}

export interface AgentPlanMessage extends BaseMessage {
  kind: MessageKind.AgentPlan
  entries: PlanEntry[]
}

export interface UnknownMessage extends BaseMessage {
  kind: MessageKind.Unknown
  raw: Record<string, unknown>
}

export type ChatMessage = UserChatMessage | AgentChatMessage | AgentThoughtMessage | AgentToolCallMessage | AgentPlanMessage | UnknownMessage

interface ChunkUpdate {
  sessionUpdate: string
  content?: ContentBlock
  messageId?: string
}

interface ToolCallUpdate {
  sessionUpdate: string
  toolCallId?: string
  title?: string
  status?: string
  kind?: string
  content?: ContentBlock[]
  rawInput?: Record<string, unknown>
  locations?: ToolCallLocation[]
}

interface PlanUpdate {
  sessionUpdate: string
  entries?: PlanEntry[]
}

function extractText(content?: ContentBlock): string {
  if (!content) {
    return ''
  }
  if (typeof content.text === 'string') {
    return content.text
  }

  return ''
}

function kindFor(sessionUpdate: string): MessageKind {
  switch (sessionUpdate) {
    case 'user_message_chunk':
      return MessageKind.User
    case 'agent_message_chunk':
      return MessageKind.AgentMessage
    case 'agent_thought_chunk':
      return MessageKind.AgentThought
    case 'tool_call':
    case 'tool_call_update':
      return MessageKind.AgentToolCall
    case 'plan':
      return MessageKind.AgentPlan
    default:
      return MessageKind.Unknown
  }
}

/**
 * Accumulates `acp:transcript` notifications into ordered message
 * variants. Chunks with the same `messageId` merge; successive
 * text/thought chunks with no `messageId` extend the previous
 * message of matching kind for the same session. Tool calls merge
 * on `toolCallId`. Everything else lands as its own entry.
 */
export function useTranscript(transcript: Ref<TranscriptEvent[]> | TranscriptEvent[], sessionId?: Ref<string>) {
  const messages = computed<ChatMessage[]>(() => {
    const events = Array.isArray(transcript) ? transcript : transcript.value
    const filter = sessionId?.value
    const acc: ChatMessage[] = []

    for (const [idx, evt] of events.entries()) {
      if (filter && evt.session_id !== filter) {
        continue
      }
      const raw = evt.update as { sessionUpdate?: string } & Record<string, unknown>
      const sessionUpdate = typeof raw.sessionUpdate === 'string' ? raw.sessionUpdate : ''
      const kind = kindFor(sessionUpdate)

      if (kind === MessageKind.Unknown) {
        acc.push({
          kind: MessageKind.Unknown,
          id: `u-${idx}`,
          sessionId: evt.session_id,
          updatedAt: idx,
          raw
        })
        continue
      }

      if (kind === MessageKind.User || kind === MessageKind.AgentMessage || kind === MessageKind.AgentThought) {
        const chunk = raw as unknown as ChunkUpdate
        const text = extractText(chunk.content)
        const hasExplicitId = typeof chunk.messageId === 'string'
        const last = acc[acc.length - 1]
        if (last && last.kind === kind && last.sessionId === evt.session_id && (hasExplicitId ? last.id === chunk.messageId : true)) {
          last.text += text
          last.updatedAt = idx
          continue
        }
        // Anon chunks key off bubble count, not event index — consecutive
        // anon chunks of the same kind+session collapse into one bubble above.
        const messageId = hasExplicitId ? (chunk.messageId as string) : `${kind}-${evt.session_id}-anon-${acc.length}`
        const base: AgentChatMessage | UserChatMessage | AgentThoughtMessage = {
          kind: kind as MessageKind.User | MessageKind.AgentMessage | MessageKind.AgentThought,
          id: messageId,
          sessionId: evt.session_id,
          updatedAt: idx,
          text
        } as AgentChatMessage | UserChatMessage | AgentThoughtMessage
        acc.push(base)
        continue
      }

      if (kind === MessageKind.AgentToolCall) {
        const tc = raw as unknown as ToolCallUpdate
        const toolCallId = tc.toolCallId ?? `tc-${idx}`
        const existing = acc.find((m) => m.kind === MessageKind.AgentToolCall && m.call.toolCallId === toolCallId) as AgentToolCallMessage | undefined
        if (existing) {
          existing.updatedAt = idx
          if (tc.title !== undefined) existing.call.title = tc.title
          if (tc.status !== undefined) existing.call.status = tc.status
          if (tc.kind !== undefined) existing.call.kind = tc.kind
          if (Array.isArray(tc.content)) existing.call.content = tc.content
          if (tc.rawInput !== undefined) existing.call.rawInput = tc.rawInput
          if (Array.isArray(tc.locations)) existing.call.locations = tc.locations
          continue
        }
        acc.push({
          kind: MessageKind.AgentToolCall,
          id: `tc-${toolCallId}`,
          sessionId: evt.session_id,
          updatedAt: idx,
          call: {
            toolCallId,
            title: tc.title,
            status: tc.status,
            kind: tc.kind,
            content: Array.isArray(tc.content) ? tc.content : [],
            rawInput: tc.rawInput,
            locations: Array.isArray(tc.locations) ? tc.locations : undefined
          }
        })
        continue
      }

      if (kind === MessageKind.AgentPlan) {
        const plan = raw as unknown as PlanUpdate
        acc.push({
          kind: MessageKind.AgentPlan,
          id: `plan-${evt.session_id}-${idx}`,
          sessionId: evt.session_id,
          updatedAt: idx,
          entries: Array.isArray(plan.entries) ? plan.entries : []
        })
      }
    }

    return acc
  })

  return { messages }
}
