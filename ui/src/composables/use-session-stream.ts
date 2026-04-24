import { ref } from 'vue'

import {
  InstanceState,
  listen,
  TauriEvent,
  type InstanceStateEventPayload,
  type PermissionRequestEventPayload,
  type TranscriptEventPayload,
  type UnlistenFn
} from '@ipc'

import { ToastTone } from '@components/types'

import { useActiveInstance, type InstanceId } from './use-active-instance'
import { pushInstanceState, resetPhaseSignals } from './use-phase'
import { pushPermissionRequest } from './use-permissions'
import { pushPlan, pushThoughtChunk } from './use-stream'
import { pushTerminalChunk } from './use-terminals'
import { pushToast } from './use-toasts'
import { pushToolCall, useTools } from './use-tools'
import { pushTranscriptChunk } from './use-transcript'

export const lastInstanceState = ref<InstanceStateEventPayload>()

function routePermission(payload: PermissionRequestEventPayload): void {
  pushPermissionRequest(payload.instance_id, payload.session_id, {
    request_id: payload.request_id,
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

interface ContentBlock {
  text?: string
}

function routeTranscript(payload: TranscriptEventPayload): void {
  const raw = payload.update as SessionUpdateEnvelope
  const kind = typeof raw.sessionUpdate === 'string' ? raw.sessionUpdate : ''
  const { instance_id: instanceId, session_id: sessionId } = payload
  switch (kind) {
    case 'user_message_chunk':
    case 'agent_message_chunk':
      pushTranscriptChunk(instanceId, sessionId, raw as Parameters<typeof pushTranscriptChunk>[2])
      return
    case 'agent_thought_chunk':
      pushThoughtChunk(instanceId, sessionId, raw as Parameters<typeof pushThoughtChunk>[2])
      return
    case 'plan':
      pushPlan(instanceId, sessionId, raw as Parameters<typeof pushPlan>[2])
      return
    case 'tool_call':
    case 'tool_call_update':
      pushToolCall(instanceId, sessionId, raw as Parameters<typeof pushToolCall>[2])
      routeTerminal(instanceId, sessionId, raw)
      return
    default:
      return
  }
}

// Terminal streams ride inside tool-call updates today (the content
// blocks carry the stdout delta). K-251's Rust-side rework may promote
// terminal chunks to their own session-update kind — at that point
// this route becomes a top-level case in routeTranscript.
function routeTerminal(instanceId: InstanceId, sessionId: string, raw: SessionUpdateEnvelope): void {
  const toolCallId = typeof raw['toolCallId'] === 'string' ? (raw['toolCallId'] as string) : undefined
  if (!toolCallId) {
    return
  }
  // `kind` only rides on the initial `tool_call`; `tool_call_update`
  // chunks carry stdout without it. Fall back to the tool store's
  // recorded kind so stdout deltas keep flowing.
  const updateKind = typeof raw['kind'] === 'string' ? (raw['kind'] as string).toLowerCase() : ''
  const recorded =
    useTools(instanceId)
      .calls.value.find((c) => c.toolCallId === toolCallId)
      ?.kind?.toLowerCase() ?? ''
  const kind = updateKind || recorded
  if (kind !== 'bash' && kind !== 'terminal') {
    return
  }
  const content = Array.isArray(raw['content']) ? (raw['content'] as ContentBlock[]) : []
  const chunk: Parameters<typeof pushTerminalChunk>[1] = {
    toolCallId,
    sessionId,
    stdout: content.map((b) => (typeof b.text === 'string' ? b.text : '')).join('')
  }
  const rawInput = raw['rawInput'] as Record<string, unknown> | undefined
  if (rawInput && typeof rawInput['command'] === 'string') {
    chunk.command = rawInput['command'] as string
  }
  if (rawInput && typeof rawInput['cwd'] === 'string') {
    chunk.cwd = rawInput['cwd'] as string
  }
  const status = typeof raw['status'] === 'string' ? (raw['status'] as string) : undefined
  if (status === 'completed' || status === 'done' || status === 'failed' || status === 'error') {
    chunk.running = false
  }
  pushTerminalChunk(instanceId, chunk)
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
      // TODO(K-254): current_mode_update subscriber goes here once the
      // Rust side emits a dedicated sessionUpdate kind for mode switches.
    }),
    await listen(TauriEvent.AcpInstanceState, (e) => {
      const { instance_id: instanceId, state } = e.payload
      lastInstanceState.value = e.payload
      pushInstanceState(instanceId, state)

      if (state === InstanceState.Running) {
        setIfUnset(instanceId)
        pushToast(ToastTone.Ok, 'session started')
      } else if (state === InstanceState.Ended && priorState.get(instanceId) === InstanceState.Running) {
        pushToast(ToastTone.Warn, 'session ended')
      }

      if (state === InstanceState.Ended || state === InstanceState.Error) {
        resetPhaseSignals(instanceId)
      }

      priorState.set(instanceId, state)
    }),
    await listen(TauriEvent.AcpPermissionRequest, (e) => {
      routePermission(e.payload)
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
