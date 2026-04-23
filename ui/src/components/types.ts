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

export enum ToolState {
  Running = 'running',
  Done = 'done',
  Failed = 'failed',
  Awaiting = 'awaiting'
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
  kind?: string
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

/** Attachment / resource pill in the composer row. */
export interface ComposerPill {
  id: string
  label: string
  kind?: string
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
 * Leading glyph per tool-kind dispatch tag on chips/rows. Kind match is
 * case-insensitive against the `kind` field; unknown or missing kinds
 * fall back to the generic `cube` glyph.
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

export function iconForToolKind(kind: string | undefined): FaIconSpec {
  return TOOL_KIND_ICONS[(kind ?? '').toLowerCase()] ?? ['fas', 'cube']
}
