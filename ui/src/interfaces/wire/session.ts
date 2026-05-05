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

/** Wire shape for the `instance_restart` Tauri command. `instanceId` is the (preserved) instance UUID. */
export interface InstanceRestartResult {
  instanceId: string
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
  /// UUID or captain-set name. Matches the JSON-RPC wire shape
  /// (`instanceId` after Tauri's camelCase pipeline).
  instanceId: string
}

export interface InstancesShutdownArgs {
  instanceId: string
}

export interface InstancesRenameArgs {
  /// UUID or current captain-set name. Daemon resolves either via
  /// `Adapter::resolve_token`.
  instanceId: string
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
  /// Optional when `ensure=true` — the daemon spawns a fresh instance
  /// from `(agentId, profileId)` if no live actor matches the id
  /// (or no id is supplied). Required when `ensure=false`.
  instanceId?: string
  /// Opt-in: when true and no live instance matches, daemon resolves
  /// `(agentId, profileId)` and bootstraps a fresh actor in-place.
  /// Drives the palette's "models / modes leaf opens on a clean
  /// overlay" path — instead of dead-ending with "no active instance",
  /// the leaf populates against a freshly-spawned actor.
  ensure?: boolean
  agentId?: string
  profileId?: string
}

export interface McpsListArgs {
  instanceId?: string
}

export interface PermissionReplyArgs {
  sessionId: string
  requestId: string
  /**
   * Real ACP option id from the agent-offered set on the originating
   * `session/request_permission` call. The captain's "remember this"
   * intent is carried by the option's typed `kind` field
   * (`allow_always` / `reject_always` write the trust store
   * automatically; `_once` variants don't); the daemon controller
   * reads the kind off the offered set when resolving. No separate
   * `remember` / `tool` / `instanceId` fields on the wire.
   */
  optionId: string
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
  /// Resolved instance id — present when the daemon ensured-spawn
  /// for the caller (see `InstanceMetaArgs.ensure`). The palette
  /// leaves use it to route follow-up `modes_set` / `models_set`
  /// commands without waiting for the registry's async auto-focus
  /// event to refresh `useActiveInstance`.
  instanceId?: string
  sessionId?: string
  cwd: string
  currentModeId?: string
  currentModelId?: string
  availableModes: { id: string; name: string; description?: string }[]
  availableModels: { id: string; name: string; description?: string }[]
}
