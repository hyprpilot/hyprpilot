import { onMounted, ref } from 'vue'

import { getProfiles, type ProfileSummary } from '@ipc'

const STORAGE_KEY = 'hyprpilot:last-profile'

/**
 * Reactive wrapper around `config/profiles`. Caches the last-used
 * profile id in `localStorage` so a reload lands on the same pick.
 * `useAdapter.profilesList()` stays as the raw one-shot IPC; this
 * composable owns the reactive ref + default resolution.
 */
export function useProfiles() {
  const profiles = ref<ProfileSummary[]>([])
  const selected = ref<string>()
  const lastErr = ref<string>()
  const loading = ref(false)

  function persist(id?: string): void {
    try {
      if (id) window.localStorage.setItem(STORAGE_KEY, id)
      else window.localStorage.removeItem(STORAGE_KEY)
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
    loading.value = true
    lastErr.value = undefined
    try {
      const list = await getProfiles()
      profiles.value = list
      if (!selected.value || !list.some((p) => p.id === selected.value)) {
        selected.value = defaultProfileId(list)
      }
    } catch (err) {
      lastErr.value = String(err)
    } finally {
      loading.value = false
    }
  }

  function select(id: string): void {
    if (!profiles.value.some((p) => p.id === id)) {
      return
    }
    selected.value = id
    persist(id)
  }

  onMounted(() => {
    void refresh()
  })

  return {
    profiles,
    selected,
    lastErr,
    loading,
    refresh,
    select
  }
}
