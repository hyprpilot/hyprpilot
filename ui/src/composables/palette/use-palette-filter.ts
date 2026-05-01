import Fuse from 'fuse.js'
import { computed, type ComputedRef, type Ref } from 'vue'

import { type PaletteEntry, PaletteMode, type PaletteSpec } from './palette'

export interface UsePaletteFilterApi {
  /// Rows to render — ticked rows pinned to the top in
  /// multi-select mode, fuzzy-filtered remainder below.
  visible: ComputedRef<PaletteEntry[]>
}

/**
 * Computes the visible row set for a palette leaf given the live
 * search `query` and the multi-select `ticked` ids. Two-stage
 * filter: subsequence-prefilter (fast, eliminates obvious non-matches)
 * then Fuse fuzzy-rank for the survivors. Empty query bypasses
 * Fuse and returns the spec's entry order verbatim.
 *
 * Pulled out of `CommandPalette.vue` so the same filter shape is
 * available to any future palette-shaped surface (an inline picker,
 * a sheet, …) without a copy-paste.
 */
export function usePaletteFilter(
  spec: Ref<PaletteSpec | undefined>,
  query: Ref<string>,
  ticked: Ref<Set<string>>
): UsePaletteFilterApi {
  const visible = computed<PaletteEntry[]>(() => {
    const s = spec.value
    if (!s) {
      return []
    }
    const q = query.value.trim()
    const tickedSet = ticked.value
    const tickedRows = s.entries.filter((e) => tickedSet.has(e.id))
    const rest = s.entries.filter((e) => !tickedSet.has(e.id))
    const gated = rest.filter((e) => subsequenceMatch(q, e.name))

    let ordered: PaletteEntry[]
    if (!q) {
      ordered = gated
    } else {
      const fuse = new Fuse(gated, { keys: ['name'], threshold: 0.5, ignoreLocation: true })
      ordered = fuse.search(q).map((r) => r.item)
    }

    if (s.mode === PaletteMode.MultiSelect) {
      return [...tickedRows, ...ordered]
    }

    return ordered
  })

  return { visible }
}

function subsequenceMatch(q: string, name: string): boolean {
  if (!q) {
    return true
  }
  const needle = q.toLowerCase()
  const hay = name.toLowerCase()
  let pos = 0
  for (const ch of needle) {
    const next = hay.indexOf(ch, pos)
    if (next < 0) {
      return false
    }
    pos = next + 1
  }

  return true
}
