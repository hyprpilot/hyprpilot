import { ref } from 'vue'

export type InstanceId = string

// Module-scoped; every call to useActiveInstance shares the same ref.
const activeId = ref<InstanceId>()

/**
 * Minimal active-instance shim for K-255. K-274 lands the full
 * palette-backed controller; until then the first running instance
 * wins and explicit `set()` always overwrites.
 */
export function useActiveInstance() {
  function set(next: InstanceId): void {
    activeId.value = next
  }

  function setIfUnset(next: InstanceId): void {
    if (!activeId.value) {
      activeId.value = next
    }
  }

  return {
    id: activeId,
    set,
    setIfUnset
  }
}
