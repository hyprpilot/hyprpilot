/**
 * Tauri event payload shapes — the `acp:*` event channel contracts.
 * Each payload mirrors the Rust `adapters::InstanceEvent` variant
 * that emits onto the matching event name.
 */
import type { TerminalChunkKind, TerminalStream } from '@constants/wire/transcript'
import type { InstanceState } from '@constants/wire/instance'
import type { PermissionOptionView, TranscriptItem } from './transcript'

export interface TranscriptEventPayload {
  agentId: string
  sessionId: string
  instanceId: string
  /// Active turn id while a `session/prompt` is in flight; `undefined`
  /// for spontaneous updates the agent emits outside any turn.
  turnId?: string
  /// Typed transcript item the UI dispatches on `kind`.
  item: TranscriptItem
}

export interface InstanceStateEventPayload {
  agentId: string
  instanceId: string
  sessionId?: string
  state: InstanceState
}

export interface PermissionRequestEventPayload {
  agentId: string
  sessionId: string
  instanceId: string
  turnId?: string
  requestId: string
  tool: string
  kind: string
  args: string
  /// Raw `tool_call.rawInput` JSON (pass-through). UI consumers
  /// extract structured fields here — `plan` for ExitPlanMode,
  /// `command` for bash, etc. — instead of re-parsing `args`.
  rawInput?: Record<string, unknown>
  /// Joined text from the tool-call's `content[]` blocks. Some
  /// agents (claude-code's `Switch mode`) ship the markdown body
  /// here instead of on `rawInput`; the modal reads as a fallback
  /// when no `rawInput` field matches the body-shape detector.
  contentText?: string
  options: PermissionOptionView[]
}

export interface TurnStartedEventPayload {
  agentId: string
  instanceId: string
  sessionId: string
  turnId: string
}

export interface TurnEndedEventPayload {
  agentId: string
  instanceId: string
  sessionId: string
  turnId: string
  /// `EndTurn` / `MaxTokens` / `MaxTurnRequests` / `Refusal` /
  /// `Cancelled` per ACP `StopReason`. `undefined` when the request
  /// errored or was cancelled by us.
  stopReason?: string
  /// ACP / transport error message when the prompt failed mid-turn
  /// (rate limit, agent crash, JSON-RPC error). UI surfaces this
  /// as a toast — without it any failure is invisible to the user.
  error?: string
}

export interface TerminalOutputChunk {
  kind: TerminalChunkKind.Output
  stream: TerminalStream
  data: string
}

export interface TerminalExitChunk {
  kind: TerminalChunkKind.Exit
  exitCode?: number
  signal?: string
}

export type TerminalChunk = TerminalOutputChunk | TerminalExitChunk

export interface TerminalEventPayload {
  agentId: string
  instanceId: string
  sessionId: string
  turnId?: string
  terminalId: string
  chunk: TerminalChunk
}

/**
 * Registry-membership delta event. Mirrors
 * `InstanceEvent::InstancesChanged` — fired on spawn / shutdown /
 * restart with the post-change membership + current focus.
 */
export interface InstancesChangedEventPayload {
  instanceIds: string[]
  focusedId?: string
}

/**
 * Focus-pointer event. Mirrors `InstanceEvent::InstancesFocused` —
 * `instanceId` is `undefined` when the registry emptied and no
 * auto-focus target exists.
 */
export interface InstancesFocusedEventPayload {
  instanceId?: string
}

/**
 * ACP `SessionInfoUpdate` notification carried as `acp:session-info-update`.
 * Per ACP spec only `title` and `updatedAt` ride here.
 */
export interface SessionInfoUpdateEventPayload {
  agentId: string
  instanceId: string
  sessionId: string
  title?: string
  updatedAt?: string
}

/**
 * ACP `CurrentModeUpdate` notification carried as `acp:current-mode-update`.
 */
export interface CurrentModeUpdateEventPayload {
  agentId: string
  instanceId: string
  sessionId: string
  currentModeId: string
}

/**
 * Daemon-side per-instance metadata refresh — `acp:instance-meta`.
 * Pushed after `session/new`, after `session/load`, and after every
 * turn ends so the header chrome resyncs even when claude-code-acp
 * doesn't proactively emit `SessionInfoUpdate` / `CurrentModeUpdate`
 * notifications.
 */
export interface InstanceMetaEventPayload {
  agentId: string
  instanceId: string
  sessionId?: string
  cwd: string
  currentModeId?: string
  currentModelId?: string
  availableModes?: { id: string; name: string; description?: string }[]
  availableModels?: { id: string; name: string; description?: string }[]
}
