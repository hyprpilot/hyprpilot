import { ref } from 'vue'

import {
  InstanceState,
  listen,
  TauriEvent,
  TerminalChunkKind,
  TranscriptItemKind,
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

/**
 * Demux typed `TranscriptItem` payloads into the per-concern composables.
 * Switch arms are exhaustive on `TranscriptItemKind`. Each arm shapes
 * the typed item into the legacy raw payload its destination
 * composable consumes (kept for backward compat with existing
 * composable internals; the wire contract is now typed).
 *
 * `Unknown` items dispatch on `wireKind` to keep session-metadata
 * variants (mode/model/title updates) flowing into `use-session-info`
 * — they're not transcript-content so they don't have typed
 * variants on `TranscriptItem` today.
 */
function routeTranscript(payload: TranscriptEventPayload): void {
  const { instanceId, sessionId, item } = payload
  switch (item.kind) {
    case TranscriptItemKind.UserPrompt:
      // Daemon-authoritative user echo — replaces the old optimistic
      // `pushTranscriptChunk` mirror in Overlay.vue.
      pushTranscriptChunk(instanceId, sessionId, {
        sessionUpdate: 'user_message_chunk',
        content: { type: 'text', text: item.text }
      } as Parameters<typeof pushTranscriptChunk>[2])
      return
    case TranscriptItemKind.UserText:
      pushTranscriptChunk(instanceId, sessionId, {
        sessionUpdate: 'user_message_chunk',
        content: { type: 'text', text: item.text }
      } as Parameters<typeof pushTranscriptChunk>[2])
      return
    case TranscriptItemKind.AgentText:
      pushTranscriptChunk(instanceId, sessionId, {
        sessionUpdate: 'agent_message_chunk',
        content: { type: 'text', text: item.text }
      } as Parameters<typeof pushTranscriptChunk>[2])
      return
    case TranscriptItemKind.AgentThought:
      pushThoughtChunk(instanceId, sessionId, {
        sessionUpdate: 'agent_thought_chunk',
        content: { type: 'text', text: item.text }
      } as Parameters<typeof pushThoughtChunk>[2])
      return
    case TranscriptItemKind.Plan:
      pushPlan(instanceId, sessionId, {
        sessionUpdate: 'plan',
        entries: item.steps
      } as Parameters<typeof pushPlan>[2])
      return
    case TranscriptItemKind.ToolCall:
      pushToolCall(instanceId, sessionId, {
        sessionUpdate: 'tool_call',
        toolCallId: item.id,
        kind: item.toolKind,
        title: item.title,
        status: item.state,
        rawInput: item.rawArgs ? { command: item.rawArgs } : undefined,
        content: item.content
      } as Parameters<typeof pushToolCall>[2])
      return
    case TranscriptItemKind.ToolCallUpdate:
      pushToolCall(instanceId, sessionId, {
        sessionUpdate: 'tool_call_update',
        toolCallId: item.id,
        kind: item.toolKind,
        title: item.title,
        status: item.state,
        rawInput: item.rawArgs ? { command: item.rawArgs } : undefined,
        content: item.content
      } as Parameters<typeof pushToolCall>[2])
      return
    case TranscriptItemKind.PermissionRequest:
      // Permission rendering rides the dedicated `acp:permission-request`
      // event channel (sticky stack today). The transcript variant
      // exists for typed completeness; UI ignores it here.
      return
    case TranscriptItemKind.Unknown:
      // Forward-compat catch-all. Today we still need to route
      // session-metadata variants (mode/model/title) into
      // `use-session-info` until they get typed transcript variants
      // of their own.
      if (
        item.wireKind === 'current_mode_update' ||
        item.wireKind === 'current_model_update' ||
        item.wireKind === 'session_info_update'
      ) {
        pushSessionInfoUpdate(instanceId, item.payload as Parameters<typeof pushSessionInfoUpdate>[1])
      }
      return
  }
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
