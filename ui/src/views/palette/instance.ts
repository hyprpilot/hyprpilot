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
import { ToastTone } from '@components'
import { type PaletteEntry, PaletteMode, type PaletteSpec, useActiveInstance, type InstanceId, usePalette, useProfiles, useRenameInstanceModal, useToasts } from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

const ACTION_NEW = 'new'
const ACTION_RENAME = 'rename'
const ACTION_SHUTDOWN = 'shutdown'

/// Mint a fresh instance UUID + flip `useActiveInstance` to it. The
/// wire-side `session/new` spawns lazily on the next `session/submit`
/// (matches the Overlay shell's `mintInstanceId()` path); we only
/// move the active pointer here. When `profileId` is set, also
/// persists the profile selection so the next submit routes through it.
function startNewInstance(profileId: string | undefined, label: string | undefined): void {
  const { set: setActive } = useActiveInstance()
  const { select } = useProfiles()
  const id: InstanceId = crypto.randomUUID()

  if (profileId) {
    select(profileId)
  }
  setActive(id)
  log.info('palette-instance: new instance staged', { instanceId: id, profileId })
  useToasts().push(ToastTone.Ok, label ? `new instance · ${label}` : 'new instance staged')
}

function buildInstanceLeafSpec(args: {
  focused?: InstanceId
  currentName?: string
  onPickNew: () => void
  onPickRename: () => void
  onPickShutdown: () => void
}): PaletteSpec {
  const entries: PaletteEntry[] = [
    {
      id: ACTION_NEW,
      name: 'new',
      description: 'spawn a fresh instance.'
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

      startNewInstance(profileId, profile?.id)
    }
  })

  // Override the title so the sub-palette reads as `instance · new`
  // — captain knows they're picking a profile to spawn, not just
  // switching the persisted selection.
  open({ ...spec, title: 'instance · new' })
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
    await invoke(TauriCommand.InstancesRename, { id: instanceId, name: wireName })
    toasts.push(ToastTone.Ok, wireName === null ? 'instance name cleared' : `renamed to ${wireName}`)

    return true
  } catch(err) {
    toasts.push(ToastTone.Err, `rename failed: ${String(err)}`)
    log.warn('palette-instance: rename failed', { instanceId, err: String(err) })

    return false
  }
}
