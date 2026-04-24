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
  agent_id: string
  profile_id?: string
  session_id?: string
  instance_id?: string
}

export interface CancelResult {
  cancelled: boolean
  reason?: string
}

export interface AgentSummary {
  id: string
  provider: string
  is_default: boolean
}

export interface ProfileSummary {
  id: string
  agent: string
  model?: string
  is_default: boolean
}

/** ACP-native `SessionInfo` shape returned by the `session_list` Tauri command. */
export interface SessionSummary {
  sessionId: string
  cwd: string
  title?: string
  updatedAt?: string
}

export interface ListSessionsArgs {
  instanceId?: string
  agentId: string
  profileId?: string
  cwd?: string
}

export interface LoadSessionArgs {
  instanceId?: string
  agentId: string
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
  option_id: string
  name: string
  kind: string
}

export interface TranscriptEventPayload {
  agent_id: string
  session_id: string
  instance_id: string
  update: Record<string, unknown>
}

export interface InstanceStateEventPayload {
  agent_id: string
  instance_id: string
  session_id?: string
  state: InstanceState
}

export interface PermissionRequestEventPayload {
  agent_id: string
  session_id: string
  instance_id: string
  request_id: string
  tool: string
  kind: string
  args: string
  options: PermissionOptionView[]
}
