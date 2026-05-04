import { computed, reactive, type ComputedRef } from 'vue'

import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import { useProfiles } from '../ui-state/use-profiles'
import type { GitStatus } from '@components'
import type { ProfileSummary } from '@ipc'

/**
 * One advertised mode option. Mirrors ACP `SessionMode` —
 * `{ id, name, description? }`. Values stay snake_case in the
 * envelope but reach us camelCased through Tauri's serde
 * configuration.
 */
export interface SessionModeOption {
  id: string
  name: string
  description?: string
}

/**
 * One advertised model option. Mirrors ACP `SessionModel` —
 * `{ id, name, description? }`. Reaches the UI as part of the
 * `current_model_update` / `session_info_update` envelope.
 */
export interface SessionModelOption {
  id: string
  name: string
  description?: string
}

/**
 * Composite UI view of a session — a flattened union of several
 * different ACP signals plus daemon-side metadata. Each field has a
 * distinct producer (see `pushX` helpers below):
 *
 *  - `title` / `updatedAt` ← ACP `SessionInfoUpdate` notification
 *  - `mode` ← ACP `CurrentModeUpdate` notification, plus the spawn-
 *    time `InstanceInfo.mode`
 *  - `availableModes` ← ACP `NewSessionResponse.modes` (one-shot at
 *    spawn — ACP has no streaming update for this list)
 *  - `model` / `availableModels` ← `NewSessionResponse.models`
 *    (`unstable_session_model` feature; one-shot at spawn)
 *  - `cwd` ← daemon-side `InstanceInfo.cwd` (set client-side at
 *    `session/new`; no ACP update notification carries cwd)
 *  - `agent` ← daemon-side `InstanceInfo.agent_id`. Lets the header
 *    surface the active agent (e.g. `claude-code`) even when no
 *    `[[profiles]]` entry exists in config — config profiles are
 *    optional, but every live instance has an agent id.
 *  - `gitStatus` ← daemon-side enrichment (probes the cwd via
 *    `git status`; non-ACP — the agent never reports git state)
 *  - `mcpsCount` ← config-side
 *  - `restored` ← daemon-side, flipped on `session_load`
 */
export interface SessionInfo {
  title?: string
  updatedAt?: string
  cwd?: string
  agent?: string
  /// Captain-set instance name (`hyprpilot ctl instances rename`).
  /// `undefined` when no name has been set. Drives the header's
  /// leftmost pill — when present it replaces the profile pill so
  /// the captain reads their own slug instead of the upstream
  /// profile id.
  name?: string
  /// Spawning profile id. Drives the header's profile pill —
  /// distinct from the user's persisted profile picker (which only
  /// changes on explicit selection, not on focus shifts). `undefined`
  /// for bare-agent spawns or before the first InstanceMeta lands.
  profileId?: string
  mode?: string
  model?: string
  availableModes: SessionModeOption[]
  availableModels: SessionModelOption[]
  mcpsCount: number
  /// Sticky tag: `true` from `setSessionRestored` onwards. Drives
  /// the header's `↻ resumed` pill.
  restored: boolean
  /// Transient: `true` while the daemon's `Bootstrap::Resume` is
  /// streaming replay events. Flips false on the first TurnEnded
  /// for the instance (the auto-cancel after load_session triggers
  /// a TurnEnded, so this clears within a turn-end of the resume
  /// completing). Drives the chat-transcript scoped <Loading>
  /// overlay during restore.
  restoring: boolean
  gitStatus?: GitStatus
}

export interface SessionInfoState {
  title?: string
  updatedAt?: string
  cwd?: string
  agent?: string
  name?: string
  profileId?: string
  mode?: string
  model?: string
  availableModes: SessionModeOption[]
  availableModels: SessionModelOption[]
  /// MCP servers wired to this instance. Pushed by the daemon on
  /// every `acp:instance-meta` event. `undefined` until the first
  /// InstanceMeta lands; `useSessionInfo` falls back to 0 then.
  mcpsCount?: number
  restored: boolean
  restoring: boolean
  gitStatus?: GitStatus
}

/**
 * ACP `SessionInfoUpdate` payload. Per
 * `agent-client-protocol-schema-0.12.0::client::SessionInfoUpdate`,
 * this notification carries only `title` and `updatedAt` — nothing
 * else. Other header signals (mode, model, cwd, …) ride on separate
 * notifications or one-shot fields; do not bundle them in here.
 */
export interface SessionInfoUpdateRaw {
  title?: string
  updatedAt?: string
}

/**
 * ACP `CurrentModeUpdate` payload (`{ currentModeId }`).
 */
export interface CurrentModeUpdateRaw {
  currentModeId: string
}

/**
 * Spawn-time mode / model state — not a streaming update. Mirrors
 * `NewSessionResponse.modes` / `NewSessionResponse.models`. Pushed
 * once per instance from the Rust side after `session/new`.
 */
export interface InstanceModeStateRaw {
  currentModeId?: string
  availableModes?: SessionModeOption[]
}

export interface InstanceModelStateRaw {
  currentModelId?: string
  availableModels?: SessionModelOption[]
}

const states = reactive(new Map<InstanceId, SessionInfoState>())

function slotFor(id: InstanceId): SessionInfoState {
  let slot = states.get(id)

  if (!slot) {
    slot = {
      availableModes: [],
      availableModels: [],
      restored: false,
      restoring: false
    }
    states.set(id, slot)
  }

  return slot
}

/**
 * Merges an ACP `SessionInfoUpdate` notification into the per-instance
 * slot. Only `title` and `updatedAt` are spec'd here — see
 * `agent-client-protocol-schema-0.12.0::client::SessionInfoUpdate`.
 * `undefined` fields are no-ops; an explicit empty string clears the
 * field so the wire can drop a stale title.
 */
export function pushSessionInfoUpdate(id: InstanceId, raw: SessionInfoUpdateRaw): void {
  const slot = slotFor(id)

  if (typeof raw.title === 'string') {
    slot.title = raw.title
  }

  if (typeof raw.updatedAt === 'string') {
    slot.updatedAt = raw.updatedAt
  }
}

/**
 * Merges an ACP `CurrentModeUpdate` notification — sets the per-
 * instance mode to `currentModeId`. The matching `availableModes`
 * list is established at spawn via `pushInstanceModeState` and isn't
 * touched here (ACP doesn't restream the list on each mode change).
 */
export function pushCurrentModeUpdate(id: InstanceId, raw: CurrentModeUpdateRaw): void {
  slotFor(id).mode = raw.currentModeId
}

/**
 * Refresh the header title from the latest user prompt.
 * claude-code-acp never proactively sends `session_info_update`,
 * so the only signal available client-side is the user prompt
 * itself. Re-deriving on every prompt produces a rolling
 * "what's the captain working on right now" header that tracks
 * the most recent context.
 *
 * If `session_info_update` later lands with a real wire title,
 * `pushSessionInfoUpdate` still wins via call-order — we treat the
 * derived title as a default the wire is welcome to override at
 * any moment.
 */
export function setSessionTitleFromPrompt(id: InstanceId, derived: string): void {
  const slot = slotFor(id)
  const trimmed = derived.trim()

  if (!trimmed) {
    return
  }
  slot.title = trimmed.length > 60 ? `${trimmed.slice(0, 60)}…` : trimmed
}

/**
 * Spawn-time mode state — `NewSessionResponse.modes`. Sets both the
 * `currentModeId` (when present) and the advertised list. Pushed
 * once per instance lifecycle by the Rust side.
 */
export function pushInstanceModeState(id: InstanceId, raw: InstanceModeStateRaw): void {
  const slot = slotFor(id)

  if (typeof raw.currentModeId === 'string') {
    slot.mode = raw.currentModeId
  }

  if (Array.isArray(raw.availableModes)) {
    slot.availableModes = raw.availableModes
  }
}

/**
 * Spawn-time model state — `NewSessionResponse.models` (unstable
 * `session_model` feature). Sets both the active model and the
 * advertised list.
 */
export function pushInstanceModelState(id: InstanceId, raw: InstanceModelStateRaw): void {
  const slot = slotFor(id)

  if (typeof raw.currentModelId === 'string') {
    slot.model = raw.currentModelId
  }

  if (Array.isArray(raw.availableModels)) {
    slot.availableModels = raw.availableModels
  }
}

/**
 * Set the per-instance cwd. ACP has no `cwd` notification — the
 * value is established at `session/new` (client-supplied) and rides
 * on `InstanceInfo.cwd`. The Rust side pushes this once per
 * spawn / cwd-change.
 */
export function setInstanceCwd(id: InstanceId, cwd: string): void {
  slotFor(id).cwd = cwd
}

/**
 * Set the per-instance MCP server count. Daemon-side resolution
 * (root `mcps` overridden by profile `mcps`) computes the count
 * once at spawn and rides on every `acp:instance-meta` event. The
 * captain reads this through the header `+N mcps` pill — anything
 * else (palette mcps leaf, diag snapshot) reads its own source.
 */
export function setInstanceMcpsCount(id: InstanceId, count: number): void {
  slotFor(id).mcpsCount = count
}

/** Set the per-instance agent id. Mirrors `InstanceInfo.agent_id`. */
export function setInstanceAgent(id: InstanceId, agent: string): void {
  slotFor(id).agent = agent
}

/**
 * Set the captain-set instance name. `undefined` clears it. Driven by
 * the `acp:instance-renamed` event AND the boot-time `instances/list`
 * seed (so already-named instances surface their slug immediately).
 */
export function setInstanceName(id: InstanceId, name: string | undefined): void {
  const slot = slotFor(id)

  if (name === undefined || name.length === 0) {
    delete slot.name
  } else {
    slot.name = name
  }
}

/**
 * Set the per-instance spawning profile id. `undefined` clears it
 * (bare-agent spawn). Pushed by the daemon on every
 * `acp:instance-meta` event so the header chrome's profile pill
 * tracks the FOCUSED instance, not the user's persisted profile
 * picker (which only updates on explicit selection).
 */
export function setInstanceProfile(id: InstanceId, profileId: string | undefined): void {
  slotFor(id).profileId = profileId
}

/**
 * Set the per-instance git status. NOT an ACP signal — the daemon
 * probes the cwd with `git status --porcelain=v2 --branch` and
 * pushes the result here. Pass `undefined` to clear (e.g. when the
 * cwd moves outside a git repo).
 */
export function setInstanceGitStatus(id: InstanceId, gitStatus: GitStatus | undefined): void {
  slotFor(id).gitStatus = gitStatus
}

/** Toggles the restored flag for an instance — `session_load` flips this true. */
export function setSessionRestored(id: InstanceId, restored: boolean): void {
  const slot = slotFor(id)

  slot.restored = restored
}

/**
 * Toggle the transient `restoring` flag — set to `true` when the
 * UI calls `loadSession` so the chat-transcript scoped <Loading>
 * shows a "replaying transcript…" overlay; flips back to `false`
 * on the first TurnEnded for the instance (the daemon's auto-cancel
 * after `session/load` triggers one within a beat of resume) or
 * when an explicit consumer calls this with `false`.
 */
export function setSessionRestoring(id: InstanceId, restoring: boolean): void {
  slotFor(id).restoring = restoring
}

export function resetSessionInfo(id: InstanceId): void {
  states.delete(id)
}

/// Resolve the human label for a mode id from the per-instance
/// `availableModes` list. Returns `undefined` when the list hasn't
/// been seeded yet (`NewSessionResponse.modes` is one-shot at spawn)
/// or when the id isn't in it; callers fall through to displaying
/// the raw id. Pure read — no reactive dep tracked.
export function lookupModeName(id: InstanceId, modeId: string): string | undefined {
  const slot = states.get(id)

  if (!slot) {
    return undefined
  }

  return slot.availableModes.find((m) => m.id === modeId)?.name
}

/// Snapshot of the per-instance current mode id BEFORE the next
/// `pushCurrentModeUpdate` overwrites it. Captured by the session-
/// stream demuxer so the mode-change banner can render
/// `mode · <prev> → <next>` instead of just `mode → <next>`. Pure
/// read — returns `undefined` when no mode has been recorded yet.
export function lookupCurrentMode(id: InstanceId): string | undefined {
  return states.get(id)?.mode
}

/**
 * Reactive read-only view over the per-instance session info.
 * `mcpsCount` derives from the active profile — wired as zero
 * placeholder until K-258 surfaces the count on `ProfileSummary`.
 * cwd / model fall back to the active profile when the instance
 * hasn't pushed an override yet.
 */
/**
 * Project a per-instance slot + the configured profile registry into
 * the public `SessionInfo` shape. Keeps the computed body thin so the
 * lint complexity gate stays inside the limit; lets unit tests assert
 * directly against the projection without standing up a composable.
 *
 * The header reads `info.profileId` to render the profile pill for
 * the focused instance — independent of the user's persisted picker
 * (`useProfiles().selected`). Falling back to the picker would
 * mis-attribute a freshly-spawned instance's profile to the last
 * manual selection. `agent` / `model` fall back through the
 * instance's OWN `profileId`, not the picker's selection.
 */
function projectSessionInfo(slot: SessionInfoState | undefined, slotProfile: ProfileSummary | undefined): SessionInfo {
  return {
    title: slot?.title,
    updatedAt: slot?.updatedAt,
    cwd: slot?.cwd,
    agent: slot?.agent ?? slotProfile?.agent,
    name: slot?.name,
    profileId: slot?.profileId,
    mode: slot?.mode,
    model: slot?.model ?? slotProfile?.model,
    availableModes: slot?.availableModes ?? [],
    availableModels: slot?.availableModels ?? [],
    mcpsCount: slot?.mcpsCount ?? 0,
    restored: slot?.restored ?? false,
    restoring: slot?.restoring ?? false,
    gitStatus: slot?.gitStatus
  }
}

/**
 * Non-reactive snapshot of an instance's session info. Useful in
 * lifecycle paths (toasts, log lines) where the caller just needs
 * the current values once and doesn't want to subscribe to a
 * computed. Returns undefined when the instance has no slot yet.
 */
export function peekSessionInfo(id: InstanceId): SessionInfo | undefined {
  const slot = states.get(id)

  if (!slot) {
    return undefined
  }

  return projectSessionInfo(slot, undefined)
}

export function useSessionInfo(instanceId?: InstanceId): {
  info: ComputedRef<SessionInfo>
} {
  const { id: activeId } = useActiveInstance()
  const { profiles } = useProfiles()

  const info = computed<SessionInfo>(() => {
    const resolvedId = instanceId ?? activeId.value
    const slot = resolvedId ? states.get(resolvedId) : undefined
    const slotProfile = slot?.profileId ? profiles.value.find((p) => p.id === slot.profileId) : undefined

    return projectSessionInfo(slot, slotProfile)
  })

  return { info }
}

/** Test-only helper. */
export function __resetAllSessionInfoForTests(): void {
  states.clear()
}
