/**
 * Typed transcript items the daemon emits via `acp:transcript`.
 * Mirrors `adapters::transcript::TranscriptItem` on the Rust side.
 * The `kind` discriminator is exhaustive — the UI demuxer should
 * switch on it and surface `Unknown` as a placeholder for forward-
 * compat with future variants.
 */
import type { FormattedToolCall } from './formatted-tool-call'
import type { Attachment } from './session'
import type { ToolCallState, TranscriptItemKind } from '@constants/wire/transcript'

export interface PermissionOptionView {
  optionId: string
  name: string
  /**
   * Wire-normalised snake-case string from the agent (`'allow_once'`,
   * `'allow_always'`, `'reject_once'`, `'reject_always'` today;
   * vendors are free to introduce new variants). The daemon only
   * classifies allow vs reject via prefix-match; other dispatch
   * keeps the string opaque so unknown vendor kinds pass through.
   * UI consumers that care about allow/reject branching should
   * prefix-match (`kind.startsWith('allow')`) for the same reason.
   */
  kind: string
}

export type ToolCallContentItem = { kind: 'text'; text: string } | { kind: 'file'; path: string; snippet?: string } | { kind: 'json'; value: unknown }

export interface ToolCallRecord {
  id: string
  /// Closed-set tool kind wire string (ACP `ToolKind`). Named
  /// `toolKind` (not `kind`) because the parent `TranscriptItem`
  /// uses `kind` as its discriminator tag.
  toolKind: string
  title: string
  state: ToolCallState
  /// Agent's raw `tool_call.rawInput` JSON object passed through
  /// verbatim. The daemon's formatter consumed this server-side; the
  /// UI keeps it for the spec-sheet "raw JSON" disclosure only.
  rawInput?: Record<string, unknown>
  content: ToolCallContentItem[]
  /// Daemon-authored presentation view. The UI renders this verbatim;
  /// no client-side formatting fallback. See
  /// `src-tauri/src/formatting/types::FormattedToolCall`.
  formatted: FormattedToolCall
  /// Wall-clock (epoch ms) of first observation; pairs with
  /// `completedAtMs` for live-tick elapsed labels on per-tool
  /// surfaces (the thinking card aggregates across kind=think calls).
  startedAtMs: number
  completedAtMs?: number
}

export interface ToolCallUpdateRecord {
  id: string
  toolKind?: string
  title?: string
  state?: ToolCallState
  rawInput?: Record<string, unknown>
  content: ToolCallContentItem[]
  /// Updated presentation view computed from merged running state.
  formatted: FormattedToolCall
  /// Re-emitted on every delta — UI doesn't need to join against the
  /// original `tool_call` record to know when the call started.
  startedAtMs: number
  completedAtMs?: number
}

/** Mirror of Rust `adapters::transcript::PlanPriority`. */
export enum PlanPriority {
  Low = 'low',
  Medium = 'medium',
  High = 'high'
}

/** Mirror of Rust `adapters::transcript::PlanStepStatus`. */
export enum PlanStepStatus {
  Pending = 'pending',
  InProgress = 'in_progress',
  Completed = 'completed'
}

export interface PlanStep {
  content: string
  priority?: PlanPriority
  status?: PlanStepStatus
}

export interface PlanRecord {
  steps: PlanStep[]
}

export interface PermissionRequestRecord {
  requestId: string
  tool: string
  toolKind: string
  args: string
  rawInput?: Record<string, unknown>
  options: PermissionOptionView[]
  /// Daemon-authored presentation view, computed via the same
  /// formatter registry the live `acp:permission-request` event
  /// uses. Carries description / fields / output the captain reads
  /// to understand what the tool will do before approving.
  formatted: FormattedToolCall
}

export type TranscriptItem =
  | { kind: TranscriptItemKind.UserPrompt; text: string; attachments: Attachment[] }
  | { kind: TranscriptItemKind.UserText; text: string }
  | { kind: TranscriptItemKind.AgentText; text: string }
  | { kind: TranscriptItemKind.AgentThought; text: string }
  /// Agent-emitted attachment — image / audio / embedded resource /
  /// resource link. Mirrors the user-side `Attachment` shape so the
  /// existing `Attachments` chat component renders them with no new
  /// renderer. Maps from `AgentMessageChunk` when the chunk's
  /// `content` block isn't text-shaped.
  | ({ kind: TranscriptItemKind.AgentAttachment } & Attachment)
  | ({ kind: TranscriptItemKind.ToolCall } & ToolCallRecord)
  | ({ kind: TranscriptItemKind.ToolCallUpdate } & ToolCallUpdateRecord)
  | ({ kind: TranscriptItemKind.Plan } & PlanRecord)
  | ({ kind: TranscriptItemKind.PermissionRequest } & PermissionRequestRecord)
  | { kind: TranscriptItemKind.Unknown; wireKind: string; payload: Record<string, unknown> }
