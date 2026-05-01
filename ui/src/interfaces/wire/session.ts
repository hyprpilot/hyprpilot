/**
 * Wire-contract shapes for the session / instance / control-plane
 * surface — Tauri `invoke` responses + arg shapes for the methods that
 * spawn, list, restart, switch, and inspect ACP instances. Mirrors the
 * Rust `adapters::*` and `rpc::*` shapes.
 */

export interface SubmitArgs {
  text: string
  attachments?: Attachment[]
  instanceId?: string
  agentId?: string
  profileId?: string
}

export interface SubmitResult {
  accepted: boolean
  agentId: string
  profileId?: string
  sessionId?: string
  instanceId?: string
}

export interface CancelArgs {
  instanceId?: string
  agentId?: string
}

export interface CancelResult {
  cancelled: boolean
  reason?: string
}

/** Wire shape for the `instance_restart` Tauri command. `id` is the (preserved) instance UUID. */
export interface InstanceRestartResult {
  id: string
}

/**
 * Per-agent static capability set. Mirrors the Rust
 * `adapters::Capabilities` struct. Populated on each `AgentSummary`
 * by the `agents/list` (and Tauri `agents_list`) wire methods so the
 * UI can gate features (resume / model-switch / mcps panel / etc.)
 * per-agent without a second roundtrip.
 */
export interface Capabilities {
  loadSession: boolean
  listSessions: boolean
  permissions: boolean
  terminals: boolean
  sessionModelSwitch: boolean
  sessionModeSwitch: boolean
  mcpsPerInstance: boolean
  listCommands: boolean
  restartWithCwd: boolean
}

export interface AgentSummary {
  id: string
  provider: string
  isDefault: boolean
  capabilities: Capabilities
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

export interface LoadSessionArgs {
  instanceId?: string
  agentId?: string
  profileId?: string
  sessionId: string
}

/**
 * A user-turn attachment delivered alongside compose text. Carries
 * binary payload (`data` base64 — for image / audio / blob types) or
 * text body (`body` — markdown / structured text). The Rust side
 * dispatches on `mime` to pick the right ACP `ContentBlock` variant.
 *
 * `slug` is the dedup key — the same attachment can't ride twice on a
 * turn even if the user picks it again.
 */
export interface Attachment {
  slug: string
  path: string
  body: string
  title?: string
  /** Base64-encoded binary payload — set for image / audio / blob attachments. */
  data?: string
  /** Explicit MIME type. Wins over extension-based detection. */
  mime?: string
}

/**
 * One row from the resolved MCP catalog as surfaced by `mcps_list`.
 * `raw` is the opaque `mcpServers` JSON entry minus the hyprpilot
 * extension key — fields like `command` / `args` / `env` / `url` /
 * vendor-specific keys live here. `hyprpilot` carries the typed
 * extension fields. `source` is the absolute path of the JSON file
 * the entry was loaded from.
 */
export interface MCPItem {
  name: string
  raw: Record<string, unknown>
  hyprpilot: {
    autoAcceptTools: string[]
    autoRejectTools: string[]
  }
  source: string
}

export interface MCPListResult {
  mcps: MCPItem[]
}

/**
 * Snapshot of one live instance, surfaced by `instances_list`. Mirrors
 * `adapters::InstanceInfo` in the Rust adapter layer (the wire shape
 * `instances/list` emits over JSON-RPC).
 */
export interface InstanceListEntry {
  agentId: string
  instanceId: string
  /// Captain-set name (slug, ≤16 chars). `undefined` until renamed.
  name?: string
  profileId?: string
  sessionId?: string
  mode?: string
}

export interface InstanceRestartArgs {
  instanceId: string
  cwd?: string
}

export interface InstancesFocusArgs {
  /// Tauri Rust side names this `id` (matches the JSON-RPC wire);
  /// keep the field name aligned so serde's camelCase pipeline picks
  /// it up unchanged.
  id: string
}

export interface InstancesShutdownArgs {
  id: string
}

export interface InstancesRenameArgs {
  /// UUID or current captain-set name. Daemon resolves either via
  /// `Adapter::resolve_token`.
  id: string
  /// New name. `null` clears the name; otherwise validated as a slug
  /// (lowercase, ≤16 chars) inside `AdapterRegistry::rename`.
  name: string | null
}

export interface InstancesRenameResult {
  instanceId: string
  name: string | null
}

export interface ModelsSetArgs {
  instanceId: string
  modelId: string
}

export interface ModesSetArgs {
  instanceId: string
  modeId: string
}

export interface InstanceMetaArgs {
  instanceId: string
}

export interface McpsListArgs {
  instanceId?: string
}

export interface PermissionReplyArgs {
  sessionId: string
  requestId: string
  /**
   * Either an ACP option id from the offered set, or one of the
   * synthetic shortcuts `'allow'` / `'deny'` that the daemon
   * resolves against the option list via
   * `pick_allow_option_id` / `pick_reject_option_id`.
   */
  optionId: string
  /**
   * Trust-store side effect tag. When set to `'allow'` / `'deny'`,
   * the daemon writes a runtime entry for `(instanceId, tool)` after
   * the wire selection lands so future calls of the same tool short-
   * circuit at decide() lane 1 without prompting. Absent / undefined
   * is "once" — wire selection happens, no persistence. The 4-button
   * UI (allow once / allow always / deny once / deny always) maps:
   * the "always" buttons set this; the "once" buttons leave it
   * undefined.
   */
  remember?: 'allow' | 'deny'
  /** Owner instance — keys the trust store alongside `tool`. */
  instanceId?: string
  /** Tool name from `tool_call.name`, used as the trust-store key. */
  tool?: string
}

export interface SessionsInfoArgs {
  id: string
}

/**
 * Snapshot returned by the `instance_meta` Tauri command — the
 * authoritative read of the daemon's per-instance metadata cache.
 * The palette pickers (modes, models) call this on every open so the
 * picker contents come from the daemon, not a UI-side mirror that may
 * lag the latest `acp:instance-meta` event.
 */
export interface InstanceMetaSnapshot {
  sessionId?: string
  cwd: string
  currentModeId?: string
  currentModelId?: string
  availableModes: { id: string; name: string; description?: string }[]
  availableModels: { id: string; name: string; description?: string }[]
}
