/**
 * Reactive wrapper around `[[profiles]]` + the daemon's currently-
 * selected default profile.
 *
 * `selected` is the captain's "next-spawn / next-restore profile"
 * pointer — daemon-singleton, mutated via `profile_set`, broadcast
 * via `acp:profile-changed`. Every connected frontend (Vue overlay,
 * nvim plugin, ctl) reads + writes through the same daemon state,
 * so cross-frontend selections stay in sync without a separate
 * coordination channel.
 *
 * **Singleton.** State lives at module scope; every call returns
 * the same refs. The first call kicks off `refresh()` (one
 * in-flight promise across all callers, so concurrent component
 * mounts don't fan out to N IPC round-trips) and `subscribe()`
 * (one `acp:profile-changed` listener for the SPA lifetime).
 *
 * Use `loading.value` to distinguish "registry hasn't fetched yet"
 * from "registry fetched, zero profiles configured" — the two
 * states look identical from `profiles.value.length` alone.
 */

import { ref, type Ref } from 'vue'

import { pushToast } from './use-toasts'
import { ToastTone } from '@components'
import { invoke, listen, TauriCommand, TauriEvent, type ProfileSummary } from '@ipc'
import { log } from '@lib'

const profiles = ref<ProfileSummary[]>([])
const selected = ref<string>()
const lastErr = ref<string>()
const loading = ref(false)
let inflight: Promise<void> | undefined
let initialised = false
let unlisten: (() => void) | undefined

async function refresh(): Promise<void> {
  if (inflight) {
    return inflight
  }
  loading.value = true
  lastErr.value = undefined
  inflight = (async() => {
    try {
      const [list, current] = await Promise.all([
        invoke(TauriCommand.ProfilesList),
        invoke(TauriCommand.ProfileGet)
      ])

      profiles.value = list.profiles

      // Daemon is the source of truth for the selected id. `null`
      // (no `[profile] default` configured AND no client has set one
      // since) collapses to `undefined` — keep the slot empty so the
      // header pill cascades to a fallback.
      selected.value = current ?? undefined
    } catch(err) {
      const message = String(err)

      lastErr.value = message
      // Surface so the user sees why the header / palette can't
      // resolve a profile — silent failure here cascades into a
      // confusing "[profile] none" header pill with no reason.
      pushToast(ToastTone.Err, `profiles list failed: ${message}`)
    } finally {
      loading.value = false
      inflight = undefined
    }
  })()

  return inflight
}

async function subscribe(): Promise<void> {
  if (unlisten) {
    return
  }

  try {
    unlisten = await listen(TauriEvent.AcpProfileChanged, (e) => {
      selected.value = e.payload.profileId
    })
  } catch(err) {
    log.warn('useProfiles: subscribe failed', { err: String(err) })
  }
}

async function select(id: string): Promise<void> {
  if (!profiles.value.some((p) => p.id === id)) {
    return
  }

  try {
    await invoke(TauriCommand.ProfileSet, { profileId: id })
    // The daemon's `acp:profile-changed` event will fire and
    // update `selected.value` — we don't write it locally first
    // (that would flicker if the daemon rejected the id, which
    // shouldn't happen given the guard above but stays
    // single-sourced through the daemon either way).
  } catch(err) {
    pushToast(ToastTone.Err, `profile set failed: ${String(err)}`)
  }
}

export interface UseProfilesApi {
  profiles: Ref<ProfileSummary[]>
  selected: Ref<string | undefined>
  lastErr: Ref<string | undefined>
  loading: Ref<boolean>
  refresh: () => Promise<void>
  select: (id: string) => Promise<void>
}

export function useProfiles(): UseProfilesApi {
  if (!initialised) {
    initialised = true
    void refresh()
    void subscribe()
  }

  return {
    profiles,
    selected,
    lastErr,
    loading,
    refresh,
    select
  }
}

/**
 * Seed the singleton from a boot snapshot. Called from
 * `applyBootSnapshot` so the first paint already has the daemon's
 * profile list + selected id — no fetch flicker.
 */
export function applyBootProfiles(list: ProfileSummary[], selectedId: string | undefined): void {
  profiles.value = list
  selected.value = selectedId
  initialised = true
}

/** Test-only hook — clears module state between vitest cases. */
export function __resetUseProfilesForTests(): void {
  profiles.value = []
  selected.value = undefined
  lastErr.value = undefined
  loading.value = false
  inflight = undefined
  initialised = false
  unlisten?.()
  unlisten = undefined
}
