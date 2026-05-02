import { ref, type Ref } from 'vue'

export interface UseMultiSelectApi {
  ticked: Ref<Set<string>>
  toggle: (id: string) => void
  isTicked: (id: string) => boolean
  reset: () => void
}

/**
 * Tickable-row state for palette multi-select leaves. Wraps a
 * reactive `Set<string>` plus the three operations consumers need:
 * toggle one id, query whether an id is ticked, reset all.
 */
export function useMultiSelect(initial?: Iterable<string>): UseMultiSelectApi {
  const ticked = ref<Set<string>>(new Set(initial ?? []))

  function toggle(id: string): void {
    const next = new Set(ticked.value)

    if (next.has(id)) {
      next.delete(id)
    } else {
      next.add(id)
    }
    ticked.value = next
  }

  function isTicked(id: string): boolean {
    return ticked.value.has(id)
  }

  function reset(): void {
    ticked.value = new Set()
  }

  return {
    ticked,
    toggle,
    isTicked,
    reset
  }
}
