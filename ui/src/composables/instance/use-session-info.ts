import { computed, reactive, type ComputedRef } from 'vue'

import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import { useProfiles } from '../ui-state/use-profiles'
import type { GitStatus } from '@components'

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
  mode?: string
  model?: string
  availableModes: SessionModeOption[]
  availableModels: SessionModelOption[]
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
 * Set the title only if the instance doesn't already carry one —
 * fallback path for agents that never emit `session_info_update`
 * (claude-code-acp at the time of writing). Called by
 * `routeTranscript` on the first `UserPrompt` so the captain has
 * *something* identifying the session in the header before the
 * agent gets around to (or never does) advertising one.
 *
 * If `session_info_update` later lands with a real title, the
 * standard `pushSessionInfoUpdate` overwrites our derived stand-in.
 */
export function setSessionTitleIfUnset(id: InstanceId, derived: string): void {
  const slot = slotFor(id)
  const trimmed = derived.trim()

  if (slot.title || !trimmed) {
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

/** Set the per-instance agent id. Mirrors `InstanceInfo.agent_id`. */
export function setInstanceAgent(id: InstanceId, agent: string): void {
  slotFor(id).agent = agent
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
 * Shorten an absolute path for header display:
 *
 * 1. `$HOME` prefix collapses to `~`.
 * 2. If still longer than `max` chars, middle-ellipsise — keep the
 *    leading `~/<top>` segment + the trailing 2 path segments,
 *    glue with `/.../`.
 *
 * Pure helper; no reactive state. `home` is injected by callers
 * (`useHomeDir().homeDir.value`) — the renderer can't read `$HOME`
 * itself, so the value comes off the `get_home_dir` Tauri command at
 * boot.
 */
export function truncateCwd(raw: string, max = 32, home?: string): string {
  let path = raw

  if (home && path.startsWith(home)) {
    path = `~${path.slice(home.length)}`
  }

  if (path.length <= max) {
    return path
  }
  const segments = path.split('/').filter((s) => s.length > 0)

  if (segments.length <= 3) {
    return path
  }
  const head = segments[0] === '~' ? '~' : `/${segments[0]}`
  const tail = segments.slice(-2).join('/')
  const middle = `${head}/.../${tail}`

  if (middle.length < path.length) {
    return middle
  }

  return path
}

/**
 * Reactive read-only view over the per-instance session info.
 * `mcpsCount` derives from the active profile — wired as zero
 * placeholder until K-258 surfaces the count on `ProfileSummary`.
 * cwd / model fall back to the active profile when the instance
 * hasn't pushed an override yet.
 */
export function useSessionInfo(instanceId?: InstanceId): {
  info: ComputedRef<SessionInfo>
} {
  const { id: activeId } = useActiveInstance()
  const { profiles, selected } = useProfiles()

  const info = computed<SessionInfo>(() => {
    const resolvedId = instanceId ?? activeId.value
    const slot = resolvedId ? states.get(resolvedId) : undefined
    const activeProfile = profiles.value.find((p) => p.id === selected.value)

    return {
      title: slot?.title,
      updatedAt: slot?.updatedAt,
      cwd: slot?.cwd,
      agent: slot?.agent ?? activeProfile?.agent,
      mode: slot?.mode,
      model: slot?.model ?? activeProfile?.model,
      availableModes: slot?.availableModes ?? [],
      availableModels: slot?.availableModels ?? [],
      mcpsCount: 0,
      restored: slot?.restored ?? false,
      restoring: slot?.restoring ?? false,
      gitStatus: slot?.gitStatus
    }
  })

  return { info }
}

/** Test-only helper. */
export function __resetAllSessionInfoForTests(): void {
  states.clear()
}
