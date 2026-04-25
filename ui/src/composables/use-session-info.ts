import { computed, reactive, type ComputedRef } from 'vue'

import { useActiveInstance, type InstanceId } from './use-active-instance'
import { useProfiles } from './use-profiles'

export interface SessionInfo {
  title?: string
  cwd?: string
  mode?: string
  model?: string
  mcpsCount: number
  skillsCount: number
  /// `true` when the active instance was created via `session_load`
  /// (the `Bootstrap::Resume` path), not `session_new`. Mirrors the
  /// pilot's "restored" indicator.
  restored: boolean
}

export interface SessionInfoState {
  title?: string
  cwd?: string
  mode?: string
  model?: string
  restored: boolean
}

export interface SessionInfoUpdateRaw {
  title?: string
  cwd?: string
  mode?: string
  /// `current_mode_update` payload uses `currentModeId`; we accept
  /// either spelling so the demuxer can forward the raw envelope.
  currentModeId?: string
  model?: string
  /// `session_info_update` carries `updatedAt` but we don't surface
  /// it today — kept here so future palette previews can read it
  /// without another wire add.
  updatedAt?: string
}

const states = reactive(new Map<InstanceId, SessionInfoState>())

function slotFor(id: InstanceId): SessionInfoState {
  let slot = states.get(id)
  if (!slot) {
    slot = { restored: false }
    states.set(id, slot)
  }

  return slot
}

/**
 * Merges a `current_mode_update` / `session_info_update` payload into
 * the per-instance slot. `undefined` fields are no-ops; an explicit
 * empty string clears the field so the wire can drop a stale value.
 * The slot is created on first push so a `setSessionRestored` call
 * against an id that has never seen an update still records cleanly.
 */
export function pushSessionInfoUpdate(id: InstanceId, raw: SessionInfoUpdateRaw): void {
  const slot = slotFor(id)
  const mode = raw.currentModeId ?? raw.mode
  if (typeof mode === 'string') {
    slot.mode = mode
  }
  if (typeof raw.title === 'string') {
    slot.title = raw.title
  }
  if (typeof raw.cwd === 'string') {
    slot.cwd = raw.cwd
  }
  if (typeof raw.model === 'string') {
    slot.model = raw.model
  }
}

/** Toggles the restored flag for an instance — `session_load` flips this true. */
export function setSessionRestored(id: InstanceId, restored: boolean): void {
  const slot = slotFor(id)
  slot.restored = restored
}

export function resetSessionInfo(id: InstanceId): void {
  states.delete(id)
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
 * `mcpsCount` / `skillsCount` derive from the active profile — wired
 * here as zero placeholders until K-258 (mcps) and K-268 (skills)
 * surface their counts on `ProfileSummary`. cwd / model fall back to
 * the active profile when the instance hasn't pushed an override yet.
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
      cwd: slot?.cwd,
      mode: slot?.mode,
      model: slot?.model ?? activeProfile?.model,
      mcpsCount: 0,
      skillsCount: 0,
      restored: slot?.restored ?? false
    }
  })

  return { info }
}

/** Test-only helper. */
export function __resetAllSessionInfoForTests(): void {
  states.clear()
}
