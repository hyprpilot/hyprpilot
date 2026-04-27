import { ref } from 'vue'

import {
  InstanceState,
  listen,
  SessionUpdateKind,
  TauriEvent,
  TerminalChunkKind,
  type InstanceStateEventPayload,
  type PermissionRequestEventPayload,
  type TerminalEventPayload,
  type TranscriptEventPayload,
  type UnlistenFn
} from '@ipc'

import { ToastTone } from '@components/types'

import { recordInstanceState, useActiveInstance, type InstanceId } from './use-active-instance'
import { pushInstanceState, resetPhaseSignals } from './use-phase'
import { pushPermissionRequest } from './use-permissions'
import { pushSessionInfoUpdate, resetSessionInfo } from './use-session-info'
import { closeTurn, pushPlan, pushThoughtChunk } from './use-stream'
import { pushTerminalChunk, pushTerminalExit } from './use-terminals'
import { pushToast } from './use-toasts'
import { pushToolCall } from './use-tools'
import { pushTranscriptChunk } from './use-transcript'
import { pushTurnEnded, pushTurnStarted } from './use-turns'

export const lastInstanceState = ref<InstanceStateEventPayload>()

function routePermission(payload: PermissionRequestEventPayload): void {
  pushPermissionRequest(payload.instanceId, payload.sessionId, {
    requestId: payload.requestId,
    tool: payload.tool,
    kind: payload.kind,
    args: payload.args,
    options: payload.options
  })
}

interface SessionUpdateEnvelope {
  sessionUpdate?: string
  [k: string]: unknown
}

function routeTranscript(payload: TranscriptEventPayload): void {
  const raw = payload.update as SessionUpdateEnvelope
  const kind = typeof raw.sessionUpdate === 'string' ? (raw.sessionUpdate as SessionUpdateKind) : undefined
  const { instanceId, sessionId } = payload
  switch (kind) {
    case SessionUpdateKind.UserMessageChunk:
    case SessionUpdateKind.AgentMessageChunk:
      pushTranscriptChunk(instanceId, sessionId, raw as Parameters<typeof pushTranscriptChunk>[2])
      return
    case SessionUpdateKind.AgentThoughtChunk:
      pushThoughtChunk(instanceId, sessionId, raw as Parameters<typeof pushThoughtChunk>[2])
      return
    case SessionUpdateKind.Plan:
      pushPlan(instanceId, sessionId, raw as Parameters<typeof pushPlan>[2])
      return
    case SessionUpdateKind.ToolCall:
    case SessionUpdateKind.ToolCallUpdate:
      pushToolCall(instanceId, sessionId, raw as Parameters<typeof pushToolCall>[2])
      bindTerminalMetadata(instanceId, raw)
      return
    case SessionUpdateKind.CurrentModeUpdate:
    case SessionUpdateKind.CurrentModelUpdate:
    case SessionUpdateKind.SessionInfoUpdate:
      pushSessionInfoUpdate(instanceId, raw as Parameters<typeof pushSessionInfoUpdate>[1])
      return
    default:
      return
  }
}

// `tool_call` updates carry the agent-side `rawInput` (with `command`
// / `cwd`) and the agent's allocated `terminal_id` when the tool is a
// terminal. We pluck just the metadata so the inline card has a
// header — stdout streaming flows through the dedicated `acp:terminal`
// path (live push from `tools::Terminals`), not these snapshots.
function bindTerminalMetadata(instanceId: InstanceId, raw: SessionUpdateEnvelope): void {
  const rawInput = raw['rawInput'] as Record<string, unknown> | undefined
  if (!rawInput) {
    return
  }
  const terminalId = pickString(rawInput, 'terminal_id', 'terminalId')
  if (!terminalId) {
    return
  }
  const command = pickString(rawInput, 'command')
  const cwd = pickString(rawInput, 'cwd')
  if (!command && !cwd) {
    return
  }
  pushTerminalChunk(instanceId, { terminalId, data: '', command, cwd })
}

function pickString(o: Record<string, unknown>, ...keys: string[]): string | undefined {
  for (const k of keys) {
    const v = o[k]
    if (typeof v === 'string') {
      return v
    }
  }

  return undefined
}

function routeTerminal(payload: TerminalEventPayload): void {
  const { instanceId, terminalId, chunk } = payload
  if (chunk.kind === TerminalChunkKind.Output) {
    pushTerminalChunk(instanceId, { terminalId, data: chunk.data })
    return
  }
  pushTerminalExit(instanceId, { terminalId, exitCode: chunk.exitCode, signal: chunk.signal })
}

/**
 * Subscribes the session-event demuxer. Resolves to an unsubscribe
 * fn that tears down every listener. Safe to call from
 * `onMounted(async () => { const stop = await startSessionStream();
 * onUnmounted(stop) })`.
 */
export async function startSessionStream(): Promise<() => void> {
  const { setIfUnset } = useActiveInstance()

  // Prior state per instance — used to suppress spurious "session ended"
  // toasts when the instance never reached running (e.g. init failure).
  const priorState = new Map<InstanceId, InstanceState>()

  const unlisteners: UnlistenFn[] = []
  unlisteners.push(
    await listen(TauriEvent.AcpTranscript, (e) => {
      routeTranscript(e.payload)
    }),
    await listen(TauriEvent.AcpInstanceState, (e) => {
      const { instanceId, agentId, state } = e.payload
      lastInstanceState.value = e.payload
      pushInstanceState(instanceId, state)
      recordInstanceState(instanceId, agentId, state)

      if (state === InstanceState.Running) {
        setIfUnset(instanceId)
        pushToast(ToastTone.Ok, 'session started')
      } else if (state === InstanceState.Ended && priorState.get(instanceId) === InstanceState.Running) {
        pushToast(ToastTone.Warn, 'session ended')
      }

      if (state === InstanceState.Ended || state === InstanceState.Error) {
        resetPhaseSignals(instanceId)
        resetSessionInfo(instanceId)
      }

      priorState.set(instanceId, state)
    }),
    await listen(TauriEvent.AcpPermissionRequest, (e) => {
      routePermission(e.payload)
    }),
    await listen(TauriEvent.AcpTurnStarted, (e) => {
      const { instanceId, sessionId, turnId } = e.payload
      // The TurnStarted signal owns the per-turn aggregation reset.
      // Each new turn opens fresh thought / plan items so chunked
      // updates within the turn merge cleanly into one block each.
      closeTurn(instanceId, sessionId)
      pushTurnStarted(instanceId, { turnId, sessionId })
    }),
    await listen(TauriEvent.AcpTurnEnded, (e) => {
      const { instanceId, sessionId, turnId, stopReason } = e.payload
      pushTurnEnded(instanceId, { turnId, sessionId, stopReason })
    }),
    await listen(TauriEvent.AcpTerminal, (e) => {
      routeTerminal(e.payload)
    })
  )

  return () => {
    for (const u of unlisteners) {
      u()
    }
    unlisteners.length = 0
    priorState.clear()
  }
}
