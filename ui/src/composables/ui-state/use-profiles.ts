import { ref, type Ref } from 'vue'

import { pushToast } from './use-toasts'
import { ToastTone } from '@components'
import { invoke, TauriCommand, type ProfileSummary } from '@ipc'

const STORAGE_KEY = 'hyprpilot:last-profile'

// Module-level singleton state — every `useProfiles()` call returns
// the same refs. The composable's previous shape created fresh
// `ref()`s per call, which made the registry invisible to non-component
// callers (e.g. `openProfilesLeaf` runs from a keyboard handler, not
// inside a Vue component, so `onMounted` never fired and the local
// refs stayed empty even though some other component had populated
// its own copy moments earlier). Singleton state means whoever
// triggers the first refresh fills the registry for everyone.
const profiles = ref<ProfileSummary[]>([])
const selected = ref<string>()
const lastErr = ref<string>()
const loading = ref(false)
let inflight: Promise<void> | undefined
let initialised = false

function persist(id?: string): void {
  try {
    if (id) {
      window.localStorage.setItem(STORAGE_KEY, id)
    } else {
      window.localStorage.removeItem(STORAGE_KEY)
    }
  } catch {
    // jsdom / private mode — don't blow up
  }
}

function readPersisted(): string | undefined {
  try {
    return window.localStorage.getItem(STORAGE_KEY) ?? undefined
  } catch {
    return undefined
  }
}

function defaultProfileId(list: ProfileSummary[]): string | undefined {
  const persisted = readPersisted()

  if (persisted && list.some((p) => p.id === persisted)) {
    return persisted
  }
  const flagged = list.find((p) => p.isDefault)

  if (flagged) {
    return flagged.id
  }

  return list[0]?.id
}

async function refresh(): Promise<void> {
  if (inflight) {
    return inflight
  }
  loading.value = true
  lastErr.value = undefined
  inflight = (async() => {
    try {
      const r = await invoke(TauriCommand.ProfilesList)
      const list = r.profiles

      profiles.value = list

      if (!selected.value || !list.some((p) => p.id === selected.value)) {
        selected.value = defaultProfileId(list)
      }
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

function select(id: string): void {
  if (!profiles.value.some((p) => p.id === id)) {
    return
  }
  selected.value = id
  persist(id)
}

/**
 * Reactive wrapper around `config/profiles`. Caches the last-used
 * profile id in `localStorage` so a reload lands on the same pick.
 *
 * **Singleton.** State lives at module scope; every call returns the
 * same refs. The first call kicks off `refresh()` (one in-flight
 * promise across all callers, so concurrent component mounts don't
 * fan out to N IPC round-trips); subsequent calls observe the same
 * registry through their reactive bindings.
 *
 * Use `loading.value` to distinguish "registry hasn't fetched yet"
 * from "registry fetched, zero profiles configured" — the two states
 * look identical from `profiles.value.length` alone.
 */
export interface UseProfilesApi {
  profiles: Ref<ProfileSummary[]>
  selected: Ref<string | undefined>
  lastErr: Ref<string | undefined>
  loading: Ref<boolean>
  refresh: () => Promise<void>
  select: (id: string) => void
}

export function useProfiles(): UseProfilesApi {
  if (!initialised) {
    initialised = true
    void refresh()
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

/** Test-only hook — clears module state between vitest cases so the
 * singleton doesn't leak across tests. Mirrors the same `__reset`
 * pattern used by `use-tools`, `use-stream`, etc. */
export function __resetUseProfilesForTests(): void {
  profiles.value = []
  selected.value = undefined
  lastErr.value = undefined
  loading.value = false
  inflight = undefined
  initialised = false
}
