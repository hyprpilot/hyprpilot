/**
 * Profiles palette leaf (K-263) — single-select. Lists `[[profiles]]`
 * via `useProfiles()` (which wraps `profiles/list`). Picking a row
 * persists the selection through `useProfiles().select` so the next
 * compose submit routes through the chosen profile, mirroring the
 * header-pill behavior. The currently-active profile renders with a
 * `(active)` kind tag.
 *
 * Out of scope: persisting `default = true` to `[agent]
 * default_profile`. The wire surface for that is `profiles/set-default`,
 * which is intentionally absent today (the daemon is restart-to-change
 * for config). When K-280 lands, the Ctrl+D delete hook below flips
 * over to it; until then it surfaces a toast and refuses.
 */

import { ToastTone } from '@components'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables'
import { useActiveInstance, useProfiles, pushToast } from '@composables'
import { log } from '@lib'

interface ProfilesLeafEntries {
  entries: PaletteEntry[]
  activeId?: string
}

interface ProfilesLeafDeps {
  list: { id: string; agent: string; model?: string; isDefault: boolean }[]
  selected?: string
  activeInstanceId?: string
}

/**
 * Pure projection — `useProfiles()` reactive state in, palette
 * entries out. Lives at module scope so the test suite can drive the
 * shape without mounting a Vue component.
 */
export function buildProfilesLeafEntries(deps: ProfilesLeafDeps): ProfilesLeafEntries {
  // Active = the user's persisted selection (drives next submit). Once
  // an instance→profile mapping ships in the UI, swap this for the
  // profile owning `useActiveInstance().id`.
  const activeId = deps.selected
  const entries: PaletteEntry[] = deps.list.map((p) => {
    const description = [p.agent, p.model ?? '—'].filter(Boolean).join(' · ')

    return {
      id: p.id,
      name: p.id,
      description,
      kind: p.id === activeId ? 'active' : p.isDefault ? 'default' : undefined,
      active: p.id === activeId
    }
  })

  return { entries, activeId }
}

export interface ProfilesPaletteSpecArgs {
  list: { id: string; agent: string; model?: string; isDefault: boolean }[]
  selected?: string
  activeInstanceId?: string
  onSelect: (id: string) => void
  /**
   * When true, picking the currently-active profile still fires
   * `onSelect`. The default (`false`) skips re-selection in the
   * profiles leaf, where committing the active row is a no-op.
   * `instance > new` flips this on so picking the active profile
   * stages a fresh instance under it.
   */
  fireOnActive?: boolean
}

/**
 * Builds the palette spec without touching `usePalette()` — keeps the
 * test path unit-pure and lets `openProfilesLeaf` handle the actual
 * stack push.
 */
export function buildProfilesPaletteSpec(args: ProfilesPaletteSpecArgs): PaletteSpec {
  const { entries, activeId } = buildProfilesLeafEntries({
    list: args.list,
    selected: args.selected,
    activeInstanceId: args.activeInstanceId
  })

  return {
    mode: PaletteMode.Select,
    title: 'profiles',
    entries,
    onCommit(picks) {
      const pick = picks[0]

      if (!pick) {
        return
      }

      if (!args.fireOnActive && pick.id === activeId) {
        return
      }
      args.onSelect(pick.id)
    },
    onDelete(entry) {
      // K-280: wire to `profiles/set-default`. Until that lands,
      // surface a toast so the keystroke is observable + refuse —
      // never fake a success.
      log.warn('palette-profiles: set-default not yet wired', { entry: entry.id })
      pushToast(ToastTone.Warn, 'set-default: not yet wired (K-280)')
    }
  }
}

/**
 * Open the profiles leaf. Reads the live `useProfiles` state, opens
 * a Select-mode palette, and on commit persists the pick via
 * `useProfiles().select` (which writes to `localStorage` + flips
 * `selected.value`, the source of truth `useAdapter().submit` reads).
 */
export function openProfilesLeaf(): void {
  const { open } = usePalette()
  const { profiles, selected, loading, select } = useProfiles()
  const { id: activeInstanceId } = useActiveInstance()

  if (profiles.value.length === 0) {
    // Distinguish "still fetching" from "fetched but empty" — both
    // look like length 0, but they're different problems for the
    // captain. Loading = wait a tick. Empty = the daemon's
    // `[[profiles]]` registry has no entries; add one to your config.
    if (loading.value) {
      pushToast(ToastTone.Warn, 'profiles: still loading, try again')
    } else {
      pushToast(ToastTone.Warn, 'profiles: none configured — add [[profiles]] to your config')
    }

    return
  }

  const spec = buildProfilesPaletteSpec({
    list: profiles.value,
    selected: selected.value,
    activeInstanceId: activeInstanceId.value,
    onSelect(id) {
      select(id)
      pushToast(ToastTone.Ok, `profile: ${id}`)
    }
  })

  open(spec)
}
