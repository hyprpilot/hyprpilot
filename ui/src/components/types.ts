/**
 * Shared typing contract for the overlay primitives (K-250). Wiring
 * issues (K-249 palette, K-251 phase machine, etc.) reuse these names;
 * do not rename without coordinating.
 */

export enum Phase {
  Idle = 'idle',
  Streaming = 'streaming',
  Pending = 'pending',
  Awaiting = 'awaiting',
  Working = 'working'
}

// Rust exposes the streaming state as `stream` (see `config.state.stream`);
// every other phase value already matches its CSS suffix 1:1.
export function phaseToCssSuffix(p: Phase): string {
  return p === Phase.Streaming ? 'stream' : p.toLowerCase()
}

export enum ToolState {
  Running = 'running',
  Done = 'done',
  Failed = 'failed',
  Awaiting = 'awaiting'
}

export function toolStateTone(state: ToolState): string {
  switch (state) {
    case ToolState.Running:
      return 'var(--theme-state-stream)'
    case ToolState.Failed:
      return 'var(--theme-status-err)'
    case ToolState.Awaiting:
      return 'var(--theme-state-awaiting)'
    case ToolState.Done:
      return 'var(--theme-status-ok)'
    default:
      return 'var(--theme-fg-dim)'
  }
}

/**
 * Closed set mirroring `[ui.theme.kind]` in `defaults.toml`. Drives both
 * the per-tool-family tint (via `var(--theme-kind-<key>)`) and the
 * big-row dispatch in `ChatToolChips`. Adding a tool family means a
 * new variant here, a new seed in `defaults.toml`, and a new entry in
 * `TOOL_KIND_ICONS` below.
 */
export enum ToolKind {
  Read = 'read',
  Write = 'write',
  Bash = 'bash',
  Search = 'search',
  Agent = 'agent',
  Think = 'think',
  Terminal = 'terminal',
  Acp = 'acp'
}

export enum ToastTone {
  Ok = 'ok',
  Warn = 'warn',
  Err = 'err',
  Info = 'info'
}

export enum ButtonTone {
  Ok = 'ok',
  Err = 'err',
  Warn = 'warn',
  Neutral = 'neutral'
}

export enum ButtonVariant {
  Solid = 'solid',
  Ghost = 'ghost'
}

export enum Role {
  User = 'user',
  Assistant = 'assistant'
}

export enum StreamKind {
  Thinking = 'thinking',
  Planning = 'planning'
}

export enum PlanStatus {
  Pending = 'pending',
  InProgress = 'in_progress',
  Completed = 'completed'
}

export interface PlanItem {
  status: PlanStatus
  text: string
}

export interface ToolChipItem {
  label: string
  arg?: string
  state: ToolState
  detail?: string
  stat?: string
  kind?: ToolKind
  /// Set when the originating tool call carries a terminal id
  /// (`rawInput.terminal_id`). Drives the inline `ChatTerminalCard`
  /// link from a Bash / Terminal chip — without this the timeline
  /// can't bind the chip back to the live stdout stream.
  terminalId?: string
}

export interface PermissionPrompt {
  id: string
  tool: string
  kind: string
  args: string
  queued?: boolean
}

export interface QueuedMessage {
  id: string
  text: string
}

export interface LiveSession {
  id: string
  title: string
  cwd: string
  adapter: string
  doing: string
  phase: Phase
}

export interface PaletteRowItem {
  id: string
  icon?: FaIconSpec
  label: string
  hint?: string
  right?: string
  danger?: boolean
}

export interface SessionPreview {
  id: string
  title: string
  cwd: string
  adapter: string
  lastActive: string
  turns: number
}

/** Breadcrumb count chip: `{ label, count, color? }`. */
export interface BreadcrumbCount {
  /// Stable identifier the consumer dispatches on (`mcps` / `skills`
  /// / `sessions` / …). Defaults to `label` when unset.
  id?: string
  label: string
  count: number
  color?: string
}

/**
 * Git status summary for the Frame cwd row. `branch` is always
 * populated when the field is set at all; `ahead` / `behind` omitted
 * (or zero) when the branch is in sync with its upstream.
 * `worktree` is the checked-out worktree name when the cwd is inside
 * a `git-worktree` checkout, else undefined.
 */
export interface GitStatus {
  branch: string
  ahead?: number
  behind?: number
  worktree?: string
}

/**
 * Composer pill kinds. Mirrors pilot.py's two-axis model: `resource`
 * pills expand inline via `#{<kind>/<slug>}` tokens at submit time;
 * `attachment` pills ride on the next turn as ACP content blocks
 * (image / audio / blob) and `data` carries the base64 payload.
 */
export enum ComposerPillKind {
  Resource = 'resource',
  Attachment = 'attachment'
}

/**
 * Attachment / resource chip in the composer row. `data` is wire
 * payload — a file path / URL for resources, base64 image bytes for
 * attachments. `mimeType` is set on attachments so the submit path
 * can map to the right ACP `ContentBlock` variant.
 */
export interface ComposerPill {
  kind: ComposerPillKind
  id: string
  label: string
  data: string
  mimeType?: string
}

/** A set of keyboard-hint chips, e.g. `↑ ↓ move`. */
export interface KbdHintSpec {
  keys: KeyLabel[]
  label: string
}

/** fontawesome icon selector: `[pack, iconName]`. */
export type FaIconSpec = ['fas' | 'far', string]

/**
 * A single keycap in a `KbdHint`. Strings render as plain text (Ctrl,
 * Esc, Ctrl+K); tuples render as a FontAwesome glyph inside the `<kbd>`
 * (up/down arrows, enter, tab, escape).
 */
export type KeyLabel = string | FaIconSpec

/** Narrow a `KeyLabel` to its `FaIconSpec` branch. */
export function isFaIcon(k: unknown): k is FaIconSpec {
  return Array.isArray(k) && k.length === 2 && (k[0] === 'fas' || k[0] === 'far') && typeof k[1] === 'string'
}

/**
 * Leading glyph per tool-kind dispatch tag on chips/rows. Keys cover
 * every `ToolKind` variant plus a couple of legacy / alias strings
 * (`edit`, `grep`) so callers that still hand a raw string through get
 * a useful icon. Unknown or missing kinds fall back to the generic
 * `cube` glyph.
 */
const TOOL_KIND_ICONS: Record<string, FaIconSpec> = {
  bash: ['fas', 'terminal'],
  write: ['fas', 'pen-to-square'],
  read: ['fas', 'file-lines'],
  edit: ['fas', 'pen'],
  search: ['fas', 'magnifying-glass'],
  grep: ['fas', 'magnifying-glass'],
  terminal: ['fas', 'terminal'],
  agent: ['fas', 'user-gear'],
  think: ['fas', 'brain'],
  acp: ['fas', 'plug']
}

export function iconForToolKind(kind: ToolKind | string | undefined): FaIconSpec {
  return TOOL_KIND_ICONS[(kind ?? '').toLowerCase()] ?? ['fas', 'cube']
}
