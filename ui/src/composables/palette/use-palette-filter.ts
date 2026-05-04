import { computed, ref, watch, type ComputedRef, type Ref } from 'vue'

import { type PaletteEntry, PaletteMode, type PaletteSpec } from './palette'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

const RANK_DEBOUNCE_MS = 60

export interface UsePaletteFilterApi {
  /// Rows to render — ticked rows pinned to the top in
  /// multi-select mode, fuzzy-ranked remainder below.
  visible: ComputedRef<PaletteEntry[]>
}

/**
 * Computes the visible row set for a palette leaf given the live
 * search `query` and the multi-select `ticked` ids. Empty query →
 * spec entry order verbatim; non-empty query → daemon-side
 * `completion/rank` (nucleo matcher), debounced so each keystroke
 * doesn't fire a new RPC. Server-pre-filtered specs (cwd
 * path-completion) opt out via `spec.filtered`.
 *
 * Ranking lives daemon-side so every frontend (Vue overlay, Neovim
 * plugin, …) shares one matcher implementation.
 */
export function usePaletteFilter(spec: Ref<PaletteSpec | undefined>, query: Ref<string>, ticked: Ref<Set<string>>): UsePaletteFilterApi {
  const ranked = ref<PaletteEntry[]>([])
  let debounceTimer: ReturnType<typeof setTimeout> | undefined
  let cancelToken = 0

  function applyIdentity(s: PaletteSpec): void {
    const tickedSet = ticked.value
    const tickedRows = s.entries.filter((e) => tickedSet.has(e.id))
    const rest = s.entries.filter((e) => !tickedSet.has(e.id))

    ranked.value = s.mode === PaletteMode.MultiSelect ? [...tickedRows, ...rest] : rest
  }

  watch(
    [spec, query, ticked],
    () => {
      if (debounceTimer !== undefined) {
        clearTimeout(debounceTimer)
      }
      const s = spec.value

      if (!s) {
        ranked.value = []

        return
      }

      // Server-pre-filtered specs (cwd path-completion) skip the
      // ranker — their entries are already pruned against the raw
      // query upstream.
      if (s.filtered) {
        applyIdentity(s)

        return
      }
      const q = query.value.trim()

      if (q.length === 0) {
        applyIdentity(s)

        return
      }

      cancelToken++
      const myToken = cancelToken

      debounceTimer = setTimeout(() => {
        void rankAndApply(s, q, myToken)
      }, RANK_DEBOUNCE_MS)
    },
    { immediate: true, deep: true }
  )

  async function rankAndApply(s: PaletteSpec, q: string, token: number): Promise<void> {
    const tickedSet = ticked.value
    const tickedRows = s.entries.filter((e) => tickedSet.has(e.id))
    const remaining = s.entries.filter((e) => !tickedSet.has(e.id))

    if (remaining.length === 0) {
      if (token === cancelToken) {
        ranked.value = tickedRows
      }

      return
    }
    const candidates = remaining.map((e) => ({
      id: e.id,
      label: e.name,
      description: e.description
    }))
    const byId = new Map(s.entries.map((e) => [e.id, e]))

    try {
      const r = await invoke(TauriCommand.CompletionRank, { query: q, candidates })

      if (token !== cancelToken) {
        return
      }
      const ordered: PaletteEntry[] = []

      for (const item of r.items) {
        const entry = byId.get(item.replacement.text)

        if (entry !== undefined) {
          ordered.push(entry)
        }
      }
      ranked.value = s.mode === PaletteMode.MultiSelect ? [...tickedRows, ...ordered] : ordered
    } catch(err) {
      log.debug('palette-filter: completion/rank failed', { err: String(err) })

      // Fall back to identity order so the captain still sees a
      // list (just unranked) on RPC failure.
      if (token === cancelToken) {
        ranked.value = s.mode === PaletteMode.MultiSelect ? [...tickedRows, ...remaining] : remaining
      }
    }
  }

  return { visible: computed(() => ranked.value) }
}
