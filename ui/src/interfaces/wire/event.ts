/**
 * Tauri event payload shapes — the `acp:*` event channel contracts.
 * Each payload mirrors the Rust `adapters::InstanceEvent` variant
 * that emits onto the matching event name.
 */
import type { PermissionOptionView, TranscriptItem } from './transcript'
import type { InstanceState } from '@constants/wire/instance'
import type { TerminalChunkKind, TerminalStream } from '@constants/wire/transcript'

export interface TranscriptEventPayload {
  agentId: string
  sessionId: string
  instanceId: string
  /// Active turn id while a `session/prompt` is in flight; `undefined`
  /// for spontaneous updates the agent emits outside any turn.
  turnId?: string
  /// Typed transcript item the UI dispatches on `kind`.
  item: TranscriptItem
  /// `_meta` envelope pass-through from the originating
  /// `session/update` notification. Vendor-specific extension
  /// payloads live here; observability surface today (no rendering
  /// consumer). Future per-vendor UI hooks plug in by reading this
  /// field without another wire change.
  meta?: Record<string, unknown>
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
  /// Raw `tool_call.content[]` blocks (ACP wire shape — `{ type:
  /// 'content' | 'diff' | 'terminal', ... }`). Some agents
  /// (claude-code's `Switch mode`) ship the markdown body here
  /// instead of on `rawInput`; the modal walks the array directly
  /// to render text / diff / terminal blocks.
  content?: Record<string, unknown>[]
  options: PermissionOptionView[]
  /// Daemon-authored presentation view. UI renders verbatim — no
  /// client-side formatting fallback.
  formatted: import('./formatted-tool-call').FormattedToolCall
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
 * Captain-set rename event. Mirrors `InstanceEvent::InstanceRenamed`.
 * `name` is `undefined` when the captain cleared the name.
 */
export interface InstanceRenamedEventPayload {
  instanceId: string
  name?: string
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
  /// Spawning profile id, when one resolved during ensure. Drives the
  /// header chrome's profile pill — distinct from the user's persisted
  /// profile picker (which only changes on explicit selection, not on
  /// focus shifts). `undefined` for bare-agent spawns.
  profileId?: string
  sessionId?: string
  cwd: string
  currentModeId?: string
  currentModelId?: string
  availableModes?: { id: string; name: string; description?: string }[]
  availableModels?: { id: string; name: string; description?: string }[]
  /// MCP server count resolved for this instance (root `mcps` overridden
  /// by per-profile `mcps`). Drives the header `+N mcps` pill.
  mcpsCount?: number
}
