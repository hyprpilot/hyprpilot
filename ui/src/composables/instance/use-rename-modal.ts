/**
 * Reactive state for the singleton "rename instance" modal. The
 * palette's `instance > rename` entry calls `openRenameInstance(...)`
 * to populate the target; `Overlay.vue` mounts the `<Modal>` whose
 * v-if reads `target` so the modal appears the moment the target
 * lands in state. Save / cancel reset the target to `undefined`,
 * unmounting the modal.
 *
 * Module-singleton — the daemon owns at most one focused instance,
 * and "rename the current one" needs at most one modal at a time.
 */

import { ref, type Ref } from 'vue'

export interface RenameInstanceTarget {
  instanceId: string
  /** Current name pre-fill for the input. `undefined` when the
   *  instance has no name yet (auto-mint UUID only). */
  currentName?: string
}

const target = ref<RenameInstanceTarget>()

export interface UseRenameInstanceModalApi {
  target: Ref<RenameInstanceTarget | undefined>
  open: (next: RenameInstanceTarget) => void
  close: () => void
}

export function useRenameInstanceModal(): UseRenameInstanceModalApi {
  return {
    target,
    open: (next) => {
      target.value = next
    },
    close: () => {
      target.value = undefined
    }
  }
}
