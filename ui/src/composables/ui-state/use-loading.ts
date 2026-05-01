import { computed, type ComputedRef, ref, type Ref } from 'vue'

/**
 * Module-scope singleton for the fullscreen boot loader. The
 * `bootDone` flag flips true after `boot()` in `main.ts` resolves;
 * `bootStatus` carries the current step's user-facing label so the
 * `<Loading mode="fullscreen">` overlay can paint a description
 * instead of a bare spinner.
 *
 * Anyone can `setBootStatus("doing X")` while boot is in progress —
 * the boot sequence updates this as it walks `applyTheme` →
 * `applyWindowState` → `loadHomeDir` →
 * `loadKeymaps` so the user follows what's happening rather than
 * staring at an inscrutable spinner during the first paint.
 */
const bootStatus = ref<string>('starting…')
const bootDone = ref(false)

export function setBootStatus(label: string): void {
  bootStatus.value = label
}

export function markBootDone(): void {
  bootDone.value = true
}

export function useBootLoading(): {
  status: Ref<string>
  done: ComputedRef<boolean>
} {
  return {
    status: bootStatus,
    done: computed(() => bootDone.value)
  }
}

/** Test-only helper. */
export function __resetBootLoadingForTests(): void {
  bootStatus.value = 'starting…'
  bootDone.value = false
}
