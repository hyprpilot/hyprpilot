import { computed, type ComputedRef } from 'vue'

import { useStream, type StreamItem } from './use-stream'
import { useTools, type WireToolCall } from './use-tools'
import { TurnRole, useTranscript, type ChatTurnItem } from './use-transcript'
import { type InstanceId } from '../chrome/use-active-instance'
import { Role } from '@components'

/**
 * Per-turn timeline grouping (S2 + S8).
 *
 * Turn = the role-tagged container the wire ships (`TurnRole.User` for
 * the captain, `TurnRole.Agent` for the pilot). Stream chunks (thoughts,
 * plans), tool calls, and turn entries carry a `turnId` minted by Rust
 * per `session/prompt`; this composable groups every matching entry
 * into one `TimelineBlock` so the chat render walks a hierarchy that
 * mirrors the user's mental model: "what happened during *this* turn".
 *
 * Items lacking a `turnId` (race-edge / system) bucket into their own
 * solo block — keyed synthetically by createdAt so each one stays
 * visible without cross-turn smearing.
 */

const KIND_ORDER = {
  turn: 0,
  stream: 1,
  tool: 2
} as const

export interface TimelineTurn {
  kind: 'turn'
  createdAt: number
  turn: ChatTurnItem
}

export interface TimelineStream {
  kind: 'stream'
  createdAt: number
  item: StreamItem
}

export interface TimelineTool {
  kind: 'tool'
  createdAt: number
  call: WireToolCall
}

export type TimelineEntry = TimelineTurn | TimelineStream | TimelineTool

export interface TimelineBlock {
  role: Role
  /// Group key — ACP turn id for assistant blocks, synthetic per-row
  /// key for user blocks (each user message stays in its own block).
  groupKey: string
  /// Set when the block represents a real ACP turn; `undefined` for
  /// user blocks and for spontaneous out-of-turn agent updates.
  turnId?: string
  startedAt: number
  streamEntries: TimelineStream[]
  /// Tool calls excluding thoughts — those route through `thoughts`
  /// so the chat surface renders them as a thinking StreamCard, not
  /// as a row in the tools-chip group.
  toolCalls: TimelineTool[]
  /// `kind === 'think'` tool calls, lifted out of `toolCalls` so the
  /// renderer can emit a dedicated thinking block per thought rather
  /// than tucking them into the generic tool-chip row.
  thoughts: TimelineTool[]
  turnEntries: TimelineTurn[]
}

function entryRole(entry: TimelineEntry): Role {
  if (entry.kind === 'turn') {
    return entry.turn.role === TurnRole.User ? Role.User : Role.Assistant
  }

  return Role.Assistant
}

function entryTurnId(entry: TimelineEntry): string | undefined {
  if (entry.kind === 'turn') {
    return entry.turn.turnId
  }

  if (entry.kind === 'stream') {
    return entry.item.turnId
  }

  return entry.call.turnId
}

export function useTimelineBlocks(instanceId?: InstanceId): {
  blocks: ComputedRef<TimelineBlock[]>
} {
  const { turns } = useTranscript(instanceId)
  const { items: streamItems } = useStream(instanceId)
  const { calls: toolCalls } = useTools(instanceId)

  const blocks = computed<TimelineBlock[]>(() => {
    const entries: TimelineEntry[] = [
      ...turns.value.map<TimelineTurn>((turn) => ({
        kind: 'turn',
        createdAt: turn.createdAt,
        turn
      })),
      ...streamItems.value.map<TimelineStream>((item) => ({
        kind: 'stream',
        createdAt: item.createdAt,
        item
      })),
      ...toolCalls.value.map<TimelineTool>((call) => ({
        kind: 'tool',
        createdAt: call.createdAt,
        call
      }))
    ]

    entries.sort((a, b) => a.createdAt - b.createdAt || KIND_ORDER[a.kind] - KIND_ORDER[b.kind])

    const out: TimelineBlock[] = []

    for (const entry of entries) {
      const role = entryRole(entry)
      const turnId = entryTurnId(entry)
      const groupKey = role === Role.Assistant && turnId !== undefined ? `turn:${turnId}` : `solo:${role}:${entry.createdAt}:${entry.kind}`
      const last = out[out.length - 1]
      let block: TimelineBlock

      if (last && last.groupKey === groupKey) {
        block = last
      } else {
        block = {
          role,
          groupKey,
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
        if (entry.call.kind?.toLowerCase() === 'think') {
          block.thoughts.push(entry)
        } else {
          block.toolCalls.push(entry)
        }
      } else {
        block.turnEntries.push(entry)
      }
    }

    return out
  })

  return { blocks }
}
