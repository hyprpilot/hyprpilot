import { onBeforeUnmount, ref, type Ref } from 'vue'

export interface UseTouchDeviceApi {
  /// True when the primary pointer is coarse — phones, tablets, the
  /// SPA running on the daemon's HTTPS bridge on a touch device.
  /// Reactive: re-evaluates on `matchMedia` changes (orientation flip,
  /// laptop dock state with an attached touchscreen, etc.).
  isCoarsePointer: Ref<boolean>
}

const QUERY = '(pointer: coarse)'

/// Reactive primary-pointer probe. Drives mobile-specific UX branches
/// that can't live in CSS — most notably the composer's Enter key, which
/// must insert a newline on touch (no Shift key on the soft keyboard)
/// while still submitting on desktop.
///
/// SSR-safe: returns a non-reactive `false` when `window` is absent so
/// vitest / dev-preview imports don't blow up.
export function useTouchDevice(): UseTouchDeviceApi {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return { isCoarsePointer: ref(false) }
  }
  const mq = window.matchMedia(QUERY)
  const isCoarsePointer = ref(mq.matches)
  const onChange = (e: MediaQueryListEvent): void => {
    isCoarsePointer.value = e.matches
  }

  mq.addEventListener('change', onChange)
  onBeforeUnmount(() => {
    mq.removeEventListener('change', onChange)
  })

  return { isCoarsePointer }
}
