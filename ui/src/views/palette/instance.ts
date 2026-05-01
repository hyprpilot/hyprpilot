/**
 * Singular `instance` palette leaf — actions on the *currently
 * focused* instance. Distinct from `instances` (plural) which lists
 * every live one for switching. Today the leaf has exactly one
 * entry, `rename`; future entries (per-instance restart, shutdown,
 * notes, …) land alongside as the corresponding wire surfaces are
 * ready.
 *
 * "No active instance" stub when nothing is focused — surfaces the
 * empty state instead of inventing one.
 */

import { ToastTone } from '@components'
import {
  type PaletteEntry,
  PaletteMode,
  useActiveInstance,
  usePalette,
  useRenameInstanceModal,
  useToasts
} from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

const ACTION_RENAME = 'rename'

export async function openInstanceLeaf(): Promise<void> {
  const { id: activeId } = useActiveInstance()
  const focused = activeId.value
  const { open } = usePalette()

  if (!focused) {
    open({
      mode: PaletteMode.Select,
      title: 'instance',
      entries: [
        {
          id: 'instance-no-active',
          name: 'no active instance',
          description: 'spawn or focus an instance first'
        }
      ],
      onCommit() {}
    })
    return
  }

  // Pre-fetch the current name so the modal's input pre-fills.
  // `instance_meta` is the right read surface; falls back to None on
  // failure so the modal still opens with an empty input.
  let currentName: string | undefined
  try {
    const meta = await invoke(TauriCommand.InstanceMeta, { instanceId: focused })
    currentName = (meta as { name?: string }).name
  } catch (err) {
    log.debug('palette-instance: instance_meta read failed', { err: String(err) })
  }

  const entries: PaletteEntry[] = [
    {
      id: ACTION_RENAME,
      name: 'rename',
      description: currentName ? `current: ${currentName}` : 'set a captain-friendly name'
    }
  ]

  open({
    mode: PaletteMode.Select,
    title: 'instance',
    entries,
    onCommit(picks) {
      const pick = picks[0]
      if (pick?.id !== ACTION_RENAME) {
        return
      }
      useRenameInstanceModal().open({ instanceId: focused, currentName })
    }
  })
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
    toasts.push(
      ToastTone.Ok,
      wireName === null ? 'instance name cleared' : `renamed to ${wireName}`
    )
    return true
  } catch (err) {
    toasts.push(ToastTone.Err, `rename failed: ${String(err)}`)
    log.warn('palette-instance: rename failed', { instanceId, err: String(err) })
    return false
  }
}
