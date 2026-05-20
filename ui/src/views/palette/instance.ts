/**
 * Singular `instance` palette leaf — actions on the *currently
 * focused* instance. Distinct from `instances` (plural) which lists
 * every live one for switching. Today the leaf carries:
 *
 *  - `new` — picks a profile from the registry, mints a fresh
 *    instance UUID, and points `useActiveInstance` at it. The wire
 *    instance spawns lazily on the next `session/submit` (matches
 *    the Overlay shell's `mintInstanceId()` flow); the palette
 *    only moves the active pointer + persists the profile pick.
 *  - `rename` — opens the rename modal for the focused instance.
 *  - `shutdown` — tears down the focused instance via
 *    `instances/shutdown`. Mirrors the `Ctrl+D` shortcut on the
 *    plural `instances` palette so captains can wind down a
 *    runaway instance without first switching to it via the
 *    plural list.
 *
 * "No active instance" suppresses `rename` + `shutdown` — `new` is
 * always available so the captain can stage an instance without
 * typing a prompt first.
 */

import { shutdownInstance } from './instances'
import { buildProfilesPaletteSpec } from './profiles'
import { buildSessionEntries } from './sessions'
import { ToastTone } from '@components'
import {
  type PaletteEntry,
  PaletteMode,
  type PaletteSpec,
  pushInstanceModelState,
  pushToast,
  setInstanceAgent,
  setInstanceCwd,
  setInstanceProfile,
  setSessionRestoring,
  useActiveInstance,
  type InstanceId,
  usePalette,
  useProfiles,
  useRenameInstanceModal,
  useToasts
} from '@composables'
import { invoke, TauriCommand, type ProfileSummary, type SessionSummary } from '@ipc'
import { log } from '@lib'

const ACTION_NEW = 'new'
const ACTION_RESTORE = 'restore'
const ACTION_RENAME = 'rename'
const ACTION_SHUTDOWN = 'shutdown'

/// Mint a fresh instance UUID + flip `useActiveInstance` to it. The
/// wire-side `session/new` spawns lazily on the next `session/submit`
/// (matches the Overlay shell's `mintInstanceId()` path); we only
/// move the active pointer here. When `profileId` is set, also
/// persists the profile selection so the next submit routes through it.
///
/// **Header pre-fill**: seed `useSessionInfo` with the bits we
/// already know from the picked `ProfileSummary` (`profileId`,
/// `agent`, optional `model`). Without this the chrome header
/// pills stay empty until the daemon's first `acp:instance-meta`
/// event lands — which only fires after the actor's `session/new`
/// resolves, often several seconds in. The real `instance-meta`
/// event overrides these seeds when it arrives (the daemon's
/// values are authoritative); the seed just covers the
/// spawn-in-flight window.
function startNewInstance(profile: ProfileSummary | undefined, label: string | undefined): void {
  const { set: setActive } = useActiveInstance()
  const { select } = useProfiles()
  const id: InstanceId = crypto.randomUUID()

  if (profile) {
    select(profile.id)
    setInstanceProfile(id, profile.id)
    setInstanceAgent(id, profile.agent)

    if (profile.model) {
      pushInstanceModelState(id, { currentModelId: profile.model, availableModels: undefined })
    }

    // Seed the cwd pill from the DAEMON-RESOLVED cwd — same resolver
    // the spawn path uses (`resolve_effective_profile`), which folds
    // root `[[patches]]` onto the base profile. Without this the
    // pre-spawn pill showed `profile.cwd` raw and patch-overridden
    // cwds (a common `personal/*` vs `work/*` pattern) only landed
    // when the actor's `session/new` resolved — disconcerting flip
    // mid-spawn. Fire-and-forget; the post-spawn meta snapshot
    // re-asserts the daemon's authoritative cwd if anything drifted.
    void invoke(TauriCommand.ResolveSpawnCwd, { profileId: profile.id })
      .then((r) => {
        if (r.cwd != null) {
          setInstanceCwd(id, r.cwd)
        }
      })
      .catch((err: unknown) => {
        log.warn('palette-instance: resolve_spawn_cwd failed; falling back to raw profile.cwd', { profileId: profile.id, err: String(err) })

        // Defensive fallback only when the resolver round-trip
        // outright failed (no daemon, transport hiccup) — the raw
        // profile.cwd is still better than an empty pill.
        if (profile.cwd) {
          setInstanceCwd(id, profile.cwd)
        }
      })
  }
  setActive(id)
  log.info('palette-instance: new instance staged', {
    instanceId: id,
    profileId: profile?.id,
    agent: profile?.agent,
    model: profile?.model
  })
  useToasts().push(ToastTone.Ok, label ? `new instance · ${label}` : 'new instance staged')
}

interface BuildInstanceLeafSpecArgs {
  focused?: InstanceId
  currentName?: string
  onPickNew: () => void
  onPickRestore: () => void
  onPickRename: () => void
  onPickShutdown: () => void
}

function buildInstanceLeafSpec(args: BuildInstanceLeafSpecArgs): PaletteSpec {
  const entries: PaletteEntry[] = [
    {
      id: ACTION_NEW,
      name: 'new',
      description: 'spawn a fresh instance.'
    },
    {
      id: ACTION_RESTORE,
      name: 'restore',
      description: 'pick a profile, then a session to resume.'
    }
  ]

  if (args.focused) {
    entries.push({
      id: ACTION_RENAME,
      name: 'rename',
      description: args.currentName ? `current: ${args.currentName}` : 'set a captain-friendly name'
    })
    entries.push({
      id: ACTION_SHUTDOWN,
      name: 'shutdown',
      description: args.currentName ? `tear down ${args.currentName}` : 'tear down the focused instance',
      // Tagged so the `instance > shutdown` row renders in the
      // err-tone slot like other destructive palette actions.
      kind: 'deny'
    })
  }

  return {
    mode: PaletteMode.Select,
    title: 'instance',
    entries,
    onCommit(picks) {
      const pick = picks[0]

      if (!pick) {
        return
      }

      if (pick.id === ACTION_NEW) {
        args.onPickNew()

        return
      }

      if (pick.id === ACTION_RESTORE) {
        args.onPickRestore()

        return
      }

      if (pick.id === ACTION_RENAME) {
        args.onPickRename()

        return
      }

      if (pick.id === ACTION_SHUTDOWN) {
        args.onPickShutdown()
      }
    }
  }
}

/// Open the profiles sub-palette under the `new` action — picking a
/// profile here both stages a new instance UUID AND persists the
/// profile selection. Empty registries surface a toast.
function openNewInstanceProfilePicker(): void {
  const { open } = usePalette()
  const { profiles, selected, loading } = useProfiles()
  const { id: activeInstanceId } = useActiveInstance()

  if (profiles.value.length === 0) {
    const message = loading.value ? 'profiles: still loading, try again' : 'profiles: none configured — add [[profiles]] to your config'

    useToasts().push(ToastTone.Warn, message)

    return
  }

  const spec = buildProfilesPaletteSpec({
    list: profiles.value,
    selected: selected.value,
    activeInstanceId: activeInstanceId.value,
    // Picking the currently-active profile is the common path for
    // "stage another instance under the same profile" — fire onSelect
    // unconditionally instead of the profiles-leaf default skip.
    fireOnActive: true,
    onSelect(profileId) {
      const profile = profiles.value.find((p) => p.id === profileId)

      startNewInstance(profile, profile?.id)
    }
  })

  // Override the title so the sub-palette reads as `instance · new`
  // — captain knows they're picking a profile to spawn, not just
  // switching the persisted selection.
  open({ ...spec, title: 'instance · new' })
}

/// Restore flow: pick a profile, then a session under that profile.
/// Two-step picker mirrors `new` (which picks a profile) but instead of
/// staging an empty instance, the second picker lists existing
/// sessions for the chosen `(agent, profile)` pair and resumes one.
function openRestoreInstanceProfilePicker(): void {
  const { open } = usePalette()
  const { profiles, selected, loading } = useProfiles()
  const { id: activeInstanceId } = useActiveInstance()

  if (profiles.value.length === 0) {
    const message = loading.value ? 'profiles: still loading, try again' : 'profiles: none configured — add [[profiles]] to your config'

    useToasts().push(ToastTone.Warn, message)

    return
  }

  const spec = buildProfilesPaletteSpec({
    list: profiles.value,
    selected: selected.value,
    activeInstanceId: activeInstanceId.value,
    // Picking the currently-active profile is the common path for
    // "restore under the same profile" — fire onSelect unconditionally
    // instead of the profiles-leaf default skip.
    fireOnActive: true,
    onSelect(profileId) {
      const profile = profiles.value.find((p) => p.id === profileId)

      if (!profile) {
        return
      }
      void openRestoreSessionPicker(profile)
    }
  })

  open({ ...spec, title: 'instance · restore' })
}

/// Second leaf of the restore flow — list sessions filtered to the
/// picked profile + Enter restores the chosen one against a freshly
/// minted instance id. Cwd / agent / profile pass through to the
/// daemon's `session_load` so the resumed actor spawns under the
/// captain's selection, not the daemon-singleton default.
async function openRestoreSessionPicker(profile: ProfileSummary): Promise<void> {
  const palette = usePalette()

  // Open the placeholder spec immediately so the captain doesn't sit
  // on a static palette while the list round-trips. Empty entries +
  // `loading: true` renders an inline <Loading> with a status pill.
  palette.open({
    mode: PaletteMode.Select,
    title: `instance · restore · ${profile.id}`,
    entries: [],
    loading: true,
    status: 'fetching session list',
    onCommit: () => {}
  })

  let sessions: SessionSummary[]

  try {
    const r = await invoke(TauriCommand.SessionList, { agentId: profile.agent, profileId: profile.id })

    sessions = r.sessions
  } catch(err) {
    log.warn('palette-instance: restore session list failed', { err })
    pushToast(ToastTone.Err, `sessions list failed: ${String(err)}`)
    palette.close()

    return
  }

  if (sessions.length === 0) {
    palette.close()
    palette.open({
      mode: PaletteMode.Select,
      title: `instance · restore · ${profile.id}`,
      entries: [
        {
          id: 'restore-sessions-empty',
          name: 'no sessions to restore under this profile.'
        }
      ],
      onCommit: () => {}
    })

    return
  }

  const entries = buildSessionEntries(sessions)
  // Build a session-id → cwd map so the commit handler can pass the
  // session's stored cwd through to `session_load` (claude-agent-acp
  // scopes sessions by cwd; resuming under the wrong cwd errors with
  // "Resource not found"). `buildSessionEntries` strips the cwd off
  // its public PaletteEntry shape — keep the side-table here.
  const cwdById = new Map<string, string>()

  for (const s of sessions) {
    cwdById.set(s.sessionId, s.cwd)
  }

  palette.close()
  palette.open({
    mode: PaletteMode.Select,
    title: `instance · restore · ${profile.id}`,
    entries,
    onCommit(picks) {
      const pick = picks[0]

      if (!pick) {
        return
      }
      const cwd = cwdById.get(pick.id)
      const target: InstanceId = crypto.randomUUID()

      // Flip the `restoring` lifecycle flag on the fresh handle so the
      // chat-transcript <Loading> overlay paints the moment the
      // captain commits. Cleared by use-session-stream on the first
      // TurnEnded for `target` (matching `useSessionHistory.load` +
      // the sessions palette leaf).
      setSessionRestoring(target, true)
      void invoke(TauriCommand.SessionLoad, {
        sessionId: pick.id,
        instanceId: target,
        cwd,
        agentId: profile.agent,
        profileId: profile.id
      }).catch((err) => {
        log.warn('palette-instance: restore load failed', { err })
        pushToast(ToastTone.Err, `session load failed: ${String(err)}`)
        setSessionRestoring(target, false)
      })
    }
  })
}

export async function openInstanceLeaf(): Promise<void> {
  const { id: activeId } = useActiveInstance()
  const focused = activeId.value
  const { open } = usePalette()

  // Pre-fetch the current name so the rename modal pre-fills. Skips
  // the round-trip when there's no focused instance — `new` is the
  // only action available in that branch and doesn't need it.
  let currentName: string | undefined

  if (focused) {
    try {
      const meta = await invoke(TauriCommand.InstanceMeta, { instanceId: focused })

      currentName = (meta as { name?: string }).name
    } catch(err) {
      log.debug('palette-instance: instance_meta read failed', { err: String(err) })
    }
  }

  const spec = buildInstanceLeafSpec({
    focused,
    currentName,
    onPickNew: openNewInstanceProfilePicker,
    onPickRestore: openRestoreInstanceProfilePicker,
    onPickRename() {
      if (!focused) {
        return
      }
      useRenameInstanceModal().open({ instanceId: focused, currentName })
    },
    onPickShutdown() {
      if (!focused) {
        return
      }
      void shutdownInstance(focused)
    }
  })

  open(spec)
}

/// Slug rule mirror — same regex `validate_instance_name` enforces
/// daemon-side. Surfaces inline error pills before the wire call so
/// the captain doesn't need a daemon round-trip to see "bad slug".
export function validateInstanceName(raw: string): string | null {
  if (raw.length === 0) {
    // Empty = clear name. Accept here; the wire path passes None.
    return null
  }

  if (raw.length > 16) {
    return `must be ≤16 chars (got ${raw.length})`
  }

  if (!/^[a-z0-9][a-z0-9_-]*$/.test(raw)) {
    return 'lowercase a-z, 0-9, "-", "_" only; must start with letter/digit'
  }

  return null
}

export async function commitInstanceRename(instanceId: string, draft: string): Promise<boolean> {
  const toasts = useToasts()
  // Empty string = clear the name. Daemon-side wire takes
  // `name: null` for clear; the trim() catches whitespace-only.
  const trimmed = draft.trim()
  const wireName = trimmed.length === 0 ? null : trimmed

  try {
    await invoke(TauriCommand.InstancesRename, { instanceId, name: wireName })
    toasts.push(ToastTone.Ok, wireName === null ? 'instance name cleared' : `renamed to ${wireName}`)

    return true
  } catch(err) {
    toasts.push(ToastTone.Err, `rename failed: ${String(err)}`)
    log.warn('palette-instance: rename failed', { instanceId, err: String(err) })

    return false
  }
}
