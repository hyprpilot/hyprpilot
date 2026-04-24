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
  has_prompt: boolean
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
  agentId: string
  profileId?: string
  cwd?: string
}

export interface LoadSessionArgs {
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
