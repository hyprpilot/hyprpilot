import { ref } from 'vue'

import { cleanupInstance } from './cleanup'
import { pushPermissionRequest } from './use-permissions'
import { pushInstanceState } from './use-phase'
import {
  pushCurrentModeUpdate,
  pushInstanceModeState,
  pushInstanceModelState,
  pushSessionInfoUpdate,
  setInstanceAgent,
  setInstanceCwd,
  setInstanceMcpsCount,
  setInstanceName,
  setInstanceProfile,
  setSessionRestoring,
  setSessionTitleFromPrompt,
  lookupCurrentMode,
  lookupModeName
} from './use-session-info'
import { closeTurn, deleteStreamByTurnId, pushModeChange, pushPlan, pushThoughtChunk } from './use-stream'
import { pushTerminalChunk, pushTerminalExit } from './use-terminals'
import { deleteToolsByTurnId, pushToolCall } from './use-tools'
import { deleteTurnByTurnId, pushTranscriptChunk } from './use-transcript'
import { pushTurnEnded, pushTurnStarted } from './use-turns'
import { recordInstanceState, useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import { pushToast } from '../ui-state/use-toasts'
import { ToastTone, CancelToastBody } from '@components'
import {
  InstanceState,
  invoke,
  listen,
  TauriCommand,
  TauriEvent,
  TerminalChunkKind,
  TranscriptItemKind,
  type InstanceStateEventPayload,
  type PermissionRequestEventPayload,
  type TerminalEventPayload,
  type TranscriptEventPayload,
  type UnlistenFn
} from '@ipc'
import { log } from '@lib'

async function seedInstanceNames(): Promise<void> {
  try {
    const r = await invoke(TauriCommand.InstancesList)

    for (const entry of r.instances) {
      if (entry.name !== undefined && entry.name.length > 0) {
        setInstanceName(entry.instanceId, entry.name)
      }
    }
  } catch(err) {
    log.warn('instance-name seed: instances_list failed', { err: String(err) })
  }
}

export const lastInstanceState = ref<InstanceStateEventPayload>()

function routePermission(payload: PermissionRequestEventPayload): void {
  log.debug('acp:permission-request received', {
    instanceId: payload.instanceId,
    requestId: payload.requestId,
    tool: payload.tool,
    hasRawInput: payload.rawInput !== undefined,
    contentBlocks: payload.content?.length ?? 0
  })
  pushPermissionRequest(payload.instanceId, payload.sessionId, {
    agentId: payload.agentId,
    requestId: payload.requestId,
    tool: payload.tool,
    kind: payload.kind,
    args: payload.args,
    rawInput: payload.rawInput,
    content: payload.content,
    options: payload.options,
    formatted: payload.formatted
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
  const { agentId, instanceId, sessionId, item } = payload

  switch (item.kind) {
    case TranscriptItemKind.UserPrompt:
      // Daemon-authoritative user echo — replaces the old optimistic
      // `pushTranscriptChunk` mirror in Overlay.vue. Attachments ride
      // alongside so the user-turn collapsable can reflect what
      // context the captain submitted with the prompt.
      pushTranscriptChunk(instanceId, sessionId, {
        sessionUpdate: 'user_message_chunk',
        content: { type: 'text', text: item.text },
        attachments: item.attachments
      } as Parameters<typeof pushTranscriptChunk>[2])
      // Re-derive the header title from each user prompt — agents
      // like claude-code-acp never push `session_info_update`, so
      // re-running on every prompt produces a rolling "what's the
      // captain working on now" title that tracks the latest
      // context. A real wire title landing later still wins via
      // `pushSessionInfoUpdate`.
      setSessionTitleFromPrompt(instanceId, item.text)

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

    case TranscriptItemKind.AgentAttachment:
      // Agent-emitted attachment — image / audio / embedded resource
      // / resource_link. Reuses the user-side `Attachment` shape so
      // the existing `Attachments` chat component renders without a
      // new surface. The `kind` discriminator is stripped at consume
      // time; the rest of the item is the Attachment payload.
      pushTranscriptChunk(instanceId, sessionId, {
        sessionUpdate: 'agent_message_chunk',
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
      pushToolCall(instanceId, agentId, sessionId, {
        sessionUpdate: 'tool_call',
        toolCallId: item.id,
        kind: item.toolKind,
        title: item.title,
        status: item.state,
        rawInput: item.rawInput,
        content: item.content,
        formatted: item.formatted
      } as Parameters<typeof pushToolCall>[3])

      return

    case TranscriptItemKind.ToolCallUpdate:
      pushToolCall(instanceId, agentId, sessionId, {
        sessionUpdate: 'tool_call_update',
        toolCallId: item.id,
        kind: item.toolKind,
        title: item.title,
        status: item.state,
        rawInput: item.rawInput,
        content: item.content,
        formatted: item.formatted
      } as Parameters<typeof pushToolCall>[3])

      return

    case TranscriptItemKind.PermissionRequest:
      // Permission rendering rides the dedicated `acp:permission-request`
      // event channel (sticky stack today). The transcript variant
      // exists for typed completeness; UI ignores it here.
      return

    case TranscriptItemKind.Unknown:
      // Forward-compat catch-all. ACP session-metadata variants
      // (`session_info_update`, `current_mode_update`) ride on
      // dedicated `acp:session-info-update` / `acp:current-mode-update`
      // events handled below — they don't appear here. Anything that
      // does land here is a wire variant the Rust mapper hasn't been
      // taught yet; surfaced so the daemon log captures the gap.
      return
  }
}

function routeTerminal(payload: TerminalEventPayload): void {
  const { instanceId, terminalId, chunk } = payload

  if (chunk.kind === TerminalChunkKind.Output) {
    pushTerminalChunk(instanceId, { terminalId, data: chunk.data })

    return
  }
  pushTerminalExit(instanceId, {
    terminalId,
    exitCode: chunk.exitCode,
    signal: chunk.signal
  })
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
      } else if (state === InstanceState.Error) {
        // Bootstrap failure path — agent subprocess crashed, ACP
        // initialize was rejected, session/new returned an error,
        // session/load against a missing id, etc. Without surfacing
        // here the user sees a silent transcript and assumes
        // "session not started" without any diagnostic. The
        // instance's actor logs the upstream error in detail; the
        // toast is the user-facing breadcrumb.
        const label = agentId.length > 0 ? agentId : 'session'

        pushToast(ToastTone.Err, `${label} failed to start — check daemon log`)
      }

      if (state === InstanceState.Ended || state === InstanceState.Error) {
        cleanupInstance(instanceId)
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
      const { instanceId, sessionId, turnId, stopReason, error } = e.payload

      pushTurnEnded(instanceId, {
        turnId,
        sessionId,
        stopReason
      })
      // First TurnEnded after `setSessionRestoring(id, true)` clears
      // the transient flag so the chat-transcript <Loading> overlay
      // tears down. Daemon's post-load `CancelNotification` triggers
      // a TurnEnded within a beat of the resume completing, so this
      // tracks the actual "replay finished" boundary.
      setSessionRestoring(instanceId, false)

      // Cancelled / errored turns leave a half-painted user prompt +
      // partial agent response in the transcript. Offer the user a
      // single-click delete affordance via a `CancelToastBody`
      // component (which composes message + delete button inside the
      // toast). Skip when stop_reason is `end_turn` (clean completion)
      // — those turns should stay.
      const cancelled = stopReason === 'cancelled'

      if (cancelled || error) {
        const removeTurn = (): void => {
          const dropped = deleteTurnByTurnId(instanceId, turnId)

          deleteToolsByTurnId(instanceId, turnId)
          deleteStreamByTurnId(instanceId, turnId)
          log.info('turn deleted', {
            instanceId,
            turnId,
            dropped
          })
        }
        const tone = error ? ToastTone.Err : ToastTone.Warn
        const message = error ? `turn failed: ${error}` : 'turn cancelled'
        const toneVar = error ? 'var(--theme-status-err)' : 'var(--theme-status-warn)'

        pushToast(tone, {
          component: CancelToastBody,
          props: {
            message,
            tone: toneVar,
            onDelete: removeTurn
          }
        })
      }
    }),
    await listen(TauriEvent.AcpTerminal, (e) => {
      routeTerminal(e.payload)
    }),
    await listen(TauriEvent.AcpSessionInfoUpdate, (e) => {
      pushSessionInfoUpdate(e.payload.instanceId, { title: e.payload.title, updatedAt: e.payload.updatedAt })
    }),
    await listen(TauriEvent.AcpCurrentModeUpdate, (e) => {
      const { instanceId, sessionId, currentModeId } = e.payload
      // Capture the OLD mode before pushCurrentModeUpdate overwrites
      // it, so the chat banner can render `mode · plan → default`
      // instead of just `mode → default`.
      const prevModeId = lookupCurrentMode(instanceId)

      pushCurrentModeUpdate(instanceId, { currentModeId })
      // Mid-turn mode flips (claude-code's plan → default after the
      // user accepts ExitPlanMode, or codex's mode shifts) deserve a
      // visible mark in the chat — without it, the header pill flips
      // silently and the only trace is in retroactive scroll-up.
      pushModeChange(instanceId, sessionId, {
        modeId: currentModeId,
        name: lookupModeName(instanceId, currentModeId),
        prevModeId,
        prevName: prevModeId ? lookupModeName(instanceId, prevModeId) : undefined
      })
    }),
    await listen(TauriEvent.AcpInstanceMeta, (e) => {
      const { agentId, instanceId, profileId, cwd, currentModeId, currentModelId, availableModes, availableModels, mcpsCount } = e.payload

      setInstanceAgent(instanceId, agentId)
      setInstanceProfile(instanceId, profileId)
      setInstanceCwd(instanceId, cwd)

      if (typeof mcpsCount === 'number') {
        setInstanceMcpsCount(instanceId, mcpsCount)
      }
      // InstanceMeta fires once per resume completion (right after
      // session/load accepts) — that's our reliable boundary to
      // tear down the chat-transcript scoped <Loading>. Clearing on
      // *every* InstanceMeta is idempotent: subsequent emissions
      // (turn-end refresh, set_mode/set_model) just hit a no-op.
      // Without this the auto-cancel fired after load_session may
      // not produce a TurnEnded (the agent had no in-flight turn
      // to cancel), and the loader would stick forever.
      setSessionRestoring(instanceId, false)

      if (availableModes && availableModes.length > 0) {
        pushInstanceModeState(instanceId, { currentModeId, availableModes })
      } else if (currentModeId) {
        pushCurrentModeUpdate(instanceId, { currentModeId })
      }

      // Mirror the modes branch — the model list comes from
      // `NewSessionResponse.models` (gated by ACP's
      // `unstable_session_model` feature). When the agent advertises
      // both list + current id we push the full state; otherwise the
      // wire still carries `currentModelId` (e.g. seeded from
      // `[[agents]] model` config) so we keep the picker's selection
      // in sync without the list.
      if (availableModels && availableModels.length > 0) {
        pushInstanceModelState(instanceId, { currentModelId, availableModels })
      } else if (currentModelId) {
        pushInstanceModelState(instanceId, { currentModelId })
      }
    }),
    await listen(TauriEvent.AcpInstanceRenamed, (e) => {
      // Captain-set name updates (`hyprpilot ctl instances rename`)
      // — `name` undefined when cleared. Drives the header's leftmost
      // pill: when present it replaces the profile pill so the captain
      // reads their own slug instead of the upstream profile id.
      setInstanceName(e.payload.instanceId, e.payload.name)
    })
  )

  // Seed names for already-running instances on overlay open. The
  // rename event only fires on changes — without this, an instance
  // renamed before the overlay attached would render with no name
  // until the captain renames it again.
  void seedInstanceNames()

  return () => {
    for (const u of unlisteners) {
      u()
    }
    unlisteners.length = 0
    priorState.clear()
  }
}
