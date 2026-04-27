/**
 * Wire-contract shapes for every Tauri `invoke` response and `listen`
 * event payload. Together with `TauriCommandResult` / `TauriEventPayload`
 * these are the single source of truth for the Rust ↔ UI contract;
 * `invoke` / `listen` in `./bridge.ts` pick the correct response /
 * payload type off the command or event name automatically, so call
 * sites drop explicit generics.
 */

// ─── Invoke responses ────────────────────────────────────────────────

export interface SubmitResult {
  accepted: boolean
  agentId: string
  profileId?: string
  sessionId?: string
  instanceId?: string
}

export interface CancelResult {
  cancelled: boolean
  reason?: string
}

/** Wire shape for the `instance_restart` Tauri command. `id` is the (preserved) instance UUID. */
export interface InstanceRestartResult {
  id: string
}

export interface AgentSummary {
  id: string
  provider: string
  isDefault: boolean
}

export interface ProfileSummary {
  id: string
  agent: string
  model?: string
  isDefault: boolean
}

/** ACP-native `SessionInfo` shape returned by the `session_list` Tauri command. */
export interface SessionSummary {
  sessionId: string
  cwd: string
  title?: string
  updatedAt?: string
}

/**
 * Single-session projection returned by the `sessions_info` Tauri
 * command. Mirrors the wire `sessions/info` RPC handler — the row
 * data plus the resolved `agentId`/`profileId` so the palette preview
 * can correlate the picked session to a known profile.
 *
 * `messageCount` is `null` until ACP exposes a per-session turn count.
 */
export interface SessionInfoResult {
  id: string
  title?: string
  cwd: string
  lastTurnAt?: string
  messageCount?: number
  agentId: string
  profileId?: string
}

export interface ListSessionsArgs {
  instanceId?: string
  agentId?: string
  profileId?: string
  cwd?: string
}

/**
 * Skill summary surfaced by `skills_list` (K-268). Slim shape — full
 * `body` + `references` stay behind `skills_get` so a registry of
 * thousands of skills doesn't ship megabytes per palette open.
 */
export interface SkillSummary {
  slug: string
  title: string
  description: string
}

/**
 * Skill body surfaced by `skills_get` (K-268). Mirrors the Rust
 * `skills_get` command response — same shape as the `skills/get`
 * socket RPC reply.
 */
export interface SkillBody {
  slug: string
  title: string
  description: string
  body: string
  path: string
  references: string[]
}

/**
 * Slash-command summary surfaced by the agent's `available_commands`
 * SessionUpdate. Returned as a list by the `commands_list` Tauri
 * command + `commands/list` RPC. The cache lands in K-251; until
 * then the call surfaces a not-implemented error and the palette
 * renders an empty state.
 */
export interface SlashCommand {
  name: string
  description?: string
}

/**
 * A user-turn attachment delivered alongside compose text. Today the
 * sole producer is the skills palette (K-268): a tick selects a skill,
 * the body is snapshotted at pick time, and the resulting `Attachment`
 * lands in `useAttachments().pending`. Submitted with the user turn,
 * the Rust side maps each entry onto an ACP `ContentBlock::Resource`
 * (`uri = "file://<path>"`, `text = body`) prepended before the prompt
 * text block.
 *
 * `slug` is the dedup key — the same skill can't ride twice on a turn
 * even if the user clicks the palette tick a second time.
 */
export interface Attachment {
  slug: string
  path: string
  body: string
  title?: string
}

/**
 * One row from the global `[[mcps]]` catalog as surfaced by the
 * `mcps_list` Tauri command. `enabled` reflects the per-instance
 * override or the resolved profile default when `instanceId` was passed
 * on the request; otherwise it's always `true`.
 */
export interface MCPItem {
  name: string
  command: string
  enabled: boolean
}

export interface MCPListResult {
  mcps: MCPItem[]
}

export interface MCPSetResult {
  restarted: boolean
}

export interface LoadSessionArgs {
  instanceId?: string
  agentId?: string
  profileId?: string
  sessionId: string
}

export enum WindowMode {
  Anchor = 'anchor',
  Center = 'center'
}

export enum Edge {
  Top = 'top',
  Right = 'right',
  Bottom = 'bottom',
  Left = 'left'
}

/**
 * Snapshot of the daemon's resolved `[daemon.window]` state. Mirrors
 * `src-tauri/src/daemon/mod.rs::WindowState`. `anchorEdge` is the edge
 * the layer-shell surface is pinned to in anchor mode; absent in center
 * mode (no screen-edge-relative chrome should render).
 */
export interface WindowState {
  mode: WindowMode
  anchorEdge?: Edge
}

/**
 * User-desktop GTK font, parsed from `gtk-font-name`. Mirrors
 * `src-tauri/src/daemon/mod.rs::GtkFont`. `null` when the GTK query
 * failed at boot.
 */
export interface GtkFont {
  family: string
  sizePt: number
}

/** A single card's painted tokens. `bg` today; future additions slot in. */
export interface Card {
  bg: string
}

/**
 * Palette tokens surfaced by the Rust config layer. Mirrors
 * `src-tauri/src/config/mod.rs::Theme`. Every leaf is `string` because
 * `defaults.toml` always loads as the first layer — the
 * `defaults_populate_every_theme_token` test pins that invariant.
 */
export interface Theme {
  font: { mono: string; sans: string }
  window: {
    default: string
    edge: string
  }
  surface: {
    default: string
    bg: string
    alt: string
    card: {
      user: Card
      assistant: Card
    }
    compose: string
    text: string
  }
  fg: {
    default: string
    ink_2: string
    dim: string
    faint: string
  }
  border: {
    default: string
    soft: string
    focus: string
  }
  accent: {
    default: string
    user: string
    user_soft: string
    assistant: string
    assistant_soft: string
  }
  state: {
    idle: string
    stream: string
    pending: string
    awaiting: string
    working: string
  }
  kind: {
    read: string
    write: string
    bash: string
    search: string
    agent: string
    think: string
    terminal: string
    acp: string
  }
  status: {
    ok: string
    warn: string
    err: string
  }
  permission: {
    bg: string
    bg_active: string
  }
}

/**
 * `[keymaps]` config tree mirror. Every leaf is a typed `Binding`
 * (`{ modifiers, key }`) — `key` is a lowercase string matching
 * `KeyboardEvent.key.toLowerCase()` (`arrowup` for the named key,
 * `a` / `?` for single-char glyphs). Nested subgroups
 * (`palette.models`, `palette.sessions`) are their own collision scope;
 * bindings only clash within the same parent struct. See
 * `src-tauri/src/config/keymaps.rs` for the Rust-side source of truth.
 */
export interface KeymapsConfig {
  chat: ChatKeymaps
  approvals: ApprovalsKeymaps
  composer: ComposerKeymaps
  palette: PaletteKeymaps
  transcript: TranscriptKeymaps
}

export enum Modifier {
  Alt = 'alt',
  Ctrl = 'ctrl',
  Meta = 'meta',
  Shift = 'shift'
}

/**
 * A single keybinding: modifier set + key token. `key` is the lowercase
 * value Rust serialises — `arrowup` / `enter` / `tab` for named keys,
 * a single glyph (`a`, `?`, `k`) for printable characters. Matched
 * against `KeyboardEvent.key.toLowerCase()` directly; `space` is the
 * one bridge (DOM emits literal `' '`). Modifier order is canonicalised
 * Rust-side at deserialize so equality is stable.
 */
export interface Binding {
  modifiers: Modifier[]
  key: string
}

export interface ChatKeymaps {
  submit: Binding
  newline: Binding
}

export interface ApprovalsKeymaps {
  allow: Binding
  deny: Binding
}

export interface ComposerKeymaps {
  paste_image: Binding
  tab_completion: Binding
  shift_tab: Binding
  history_up: Binding
  history_down: Binding
}

export interface PaletteKeymaps {
  open: Binding
  close: Binding
  models: ModelsSubPaletteKeymaps
  sessions: SessionsSubPaletteKeymaps
}

export interface ModelsSubPaletteKeymaps {
  focus: Binding
}

export interface SessionsSubPaletteKeymaps {
  focus: Binding
}

export type TranscriptKeymaps = Record<string, never>

// ─── Event payloads ──────────────────────────────────────────────────

export enum InstanceState {
  Starting = 'starting',
  Running = 'running',
  Ended = 'ended',
  Error = 'error'
}

export interface PermissionOptionView {
  optionId: string
  name: string
  kind: string
}

/**
 * Discriminator for ACP `SessionUpdate` envelopes — the values inside
 * `update.sessionUpdate` on every `acp:transcript` event payload. The
 * variant strings stay snake_case because they're ACP wire literals:
 * `agent-client-protocol-schema` mandates snake_case for the
 * `sessionUpdate` discriminator and we don't rename them on the way
 * through. The enum gives consumers TS-compile-time enforcement on
 * the demuxer's switch arms.
 */
export enum SessionUpdateKind {
  UserMessageChunk = 'user_message_chunk',
  AgentMessageChunk = 'agent_message_chunk',
  AgentThoughtChunk = 'agent_thought_chunk',
  Plan = 'plan',
  ToolCall = 'tool_call',
  ToolCallUpdate = 'tool_call_update',
  CurrentModeUpdate = 'current_mode_update',
  CurrentModelUpdate = 'current_model_update',
  SessionInfoUpdate = 'session_info_update'
}

export interface TranscriptEventPayload {
  agentId: string
  sessionId: string
  instanceId: string
  /// Active turn id while a `session/prompt` is in flight; `undefined`
  /// for spontaneous updates the agent emits outside any turn.
  turnId?: string
  update: Record<string, unknown>
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
}

/**
 * Live terminal-stream event. Mirrors `adapters::InstanceEvent::Terminal`
 * — the Rust runtime pushes one of these per stdout / stderr chunk
 * and once on exit. UI accumulates output deltas into a
 * per-`terminalId` scrollable card; exit chunks flip the card to its
 * terminal state.
 */
export enum TerminalChunkKind {
  Output = 'output',
  Exit = 'exit'
}

export enum TerminalStream {
  Stdout = 'stdout',
  Stderr = 'stderr'
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
