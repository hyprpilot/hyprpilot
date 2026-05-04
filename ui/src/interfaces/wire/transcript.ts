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
}

export interface PlanStep {
  content: string
  priority?: string
  status?: string
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
