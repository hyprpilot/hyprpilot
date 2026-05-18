<script setup lang="ts">
/**
 * Custom scrollbar overlay for the chat viewport.
 *
 * Native scrollbar is suppressed via `scrollbar-width: none` +
 * `::-webkit-scrollbar { display: none }` in `styles.css`. This
 * component renders the replacement: a thin vertical track on the
 * right edge with a thumb whose position + size mirror the captain's
 * scroll position.
 *
 * **Position math is pixel-based** (`scrollTop / scrollHeight`), NOT
 * seq-based. Reason: seq isn't visually proportional — one long
 * streaming turn consumes many seq values while occupying little
 * vertical height. With the anchor primitive keeping `scrollTop`
 * honest under content growth (re-locked on every resize), the
 * pixel ratio IS the captain's visual position.
 *
 * **Interactions**: pointerdown on the thumb starts a drag — pointer
 * Y delta maps to a `scrollTop` write. Click on the track jumps to
 * the requested position. Native scrollbar drag was previously the
 * only way to scroll mid-history quickly; suppressing it without
 * replacement would regress that workflow.
 *
 * **Auto-hide**: track + thumb fade out 1.5s after the last scroll
 * activity (matching native overlay scrollbar behaviour on mobile)
 * unless the pointer is hovering. Captain can always force visible
 * by hovering the right edge.
 */
import { useEventListener } from '@vueuse/core'
import { computed, onMounted, onUnmounted, ref, type Ref } from 'vue'

const props = defineProps<{
  /// Scroll container — the chat transcript element. We read
  /// scrollTop / scrollHeight / clientHeight from it and write
  /// scrollTop on drag.
  scrollEl: Ref<HTMLElement | undefined> | HTMLElement | undefined
  /// Pass-through from `useStickToBottom`. When stuck the thumb
  /// pins to the bottom regardless of any sub-pixel drift in
  /// `scrollTop`.
  stuck?: boolean
}>()

const trackEl = ref<HTMLElement>()
// Tracked separately from props.scrollEl so the computed re-runs
// when the captain scrolls (scrollTop is not a reactive property of
// the element). Bumped on every scroll / resize event.
const tick = ref(0)
// Mirror prop into a ref so the computed handles both `Ref<el>` and
// `el` shapes uniformly.
const scrollRef = computed<HTMLElement | undefined>(() => {
  const sel = props.scrollEl

  if (!sel) {
    return undefined
  }

  if ('value' in sel) {
    return sel.value
  }

  return sel
})

const visible = ref(false)
let hideTimer: ReturnType<typeof setTimeout> | undefined

function showThenScheduleHide(): void {
  visible.value = true

  if (hideTimer !== undefined) {
    clearTimeout(hideTimer)
  }
  hideTimer = setTimeout(() => {
    if (!hovering.value && !dragging.value) {
      visible.value = false
    }
  }, 1500)
}

const hovering = ref(false)
const dragging = ref(false)

function bump(): void {
  tick.value += 1
  showThenScheduleHide()
}

useEventListener(scrollRef, 'scroll', bump, { passive: true })

let resizeObs: ResizeObserver | undefined

onMounted(() => {
  const el = scrollRef.value

  if (!el) {
    return
  }

  if (typeof ResizeObserver !== 'undefined') {
    resizeObs = new ResizeObserver(bump)
    resizeObs.observe(el, { box: 'content-box' })
  }
})

onUnmounted(() => {
  resizeObs?.disconnect()

  if (hideTimer !== undefined) {
    clearTimeout(hideTimer)
  }
})

const TRACK_MIN_THUMB_PX = 32
const TRACK_PADDING_PX = 2

interface ThumbMetrics {
  topPx: number
  heightPx: number
  trackHeight: number
  scrollable: boolean
}

const thumb = computed<ThumbMetrics>(() => {
  // Force re-run on every scroll / resize via the tick.
  void tick.value
  const el = scrollRef.value
  const track = trackEl.value

  if (!el || !track) {
    return {
      topPx: 0, heightPx: 0, trackHeight: 0, scrollable: false
    }
  }
  const trackHeight = track.clientHeight - TRACK_PADDING_PX * 2
  const scrollHeight = el.scrollHeight
  const clientHeight = el.clientHeight

  if (scrollHeight <= clientHeight || trackHeight <= 0) {
    return {
      topPx: 0, heightPx: 0, trackHeight, scrollable: false
    }
  }
  const ratio = clientHeight / scrollHeight
  const heightPx = Math.max(TRACK_MIN_THUMB_PX, trackHeight * ratio)
  // When thumb min clamp kicks in, we shorten the moveable distance
  // to (trackHeight - heightPx) so the thumb's top reaches the
  // bottom edge when scrolled fully down. Without this the math
  // overshoots and the thumb would scroll past the track's bottom.
  const movableTrack = trackHeight - heightPx
  const scrollProgress = el.scrollTop / (scrollHeight - clientHeight)
  // stuck=true pins to bottom even if scrollTop is slightly off
  // due to sub-pixel drift in the foot-follow rAF.
  const pinned = props.stuck === true ? 1 : Math.max(0, Math.min(1, scrollProgress))
  const topPx = TRACK_PADDING_PX + pinned * movableTrack

  return {
    topPx, heightPx, trackHeight, scrollable: true
  }
})

// ── Drag interaction ────────────────────────────────────────────
//
// Pointerdown on the thumb captures the pointer + starts a drag.
// Pointermove translates the Y delta to a `scrollTop` delta on the
// scroll element. Pointerup releases.

interface DragState {
  pointerId: number
  startPointerY: number
  startThumbTop: number
}
let drag: DragState | undefined

function onThumbPointerDown(ev: PointerEvent): void {
  const el = scrollRef.value

  if (!el || !thumb.value.scrollable) {
    return
  }
  ev.preventDefault()
  ev.stopPropagation()
  ;(ev.target as HTMLElement).setPointerCapture(ev.pointerId)
  drag = {
    pointerId: ev.pointerId,
    startPointerY: ev.clientY,
    startThumbTop: thumb.value.topPx
  }
  dragging.value = true
}

function onThumbPointerMove(ev: PointerEvent): void {
  const d = drag

  if (!d || ev.pointerId !== d.pointerId) {
    return
  }
  const el = scrollRef.value
  const t = thumb.value

  if (!el || !t.scrollable) {
    return
  }
  const movableTrack = t.trackHeight - t.heightPx

  if (movableTrack <= 0) {
    return
  }
  const deltaY = ev.clientY - d.startPointerY
  const newTopPx = Math.max(TRACK_PADDING_PX, Math.min(movableTrack + TRACK_PADDING_PX, d.startThumbTop + deltaY))
  const progress = (newTopPx - TRACK_PADDING_PX) / movableTrack
  const scrollHeight = el.scrollHeight
  const clientHeight = el.clientHeight

  el.scrollTop = progress * (scrollHeight - clientHeight)
}

function onThumbPointerUp(ev: PointerEvent): void {
  if (!drag || ev.pointerId !== drag.pointerId) {
    return
  }
  ;(ev.target as HTMLElement).releasePointerCapture(ev.pointerId)
  drag = undefined
  dragging.value = false
}

// ── Track click interaction ─────────────────────────────────────
//
// Click anywhere on the track that's NOT the thumb → jump to that
// scroll position.

function onTrackPointerDown(ev: PointerEvent): void {
  // Filter clicks on the thumb itself (handled by onThumbPointerDown).
  if ((ev.target as HTMLElement).classList.contains('chat-scrollbar-thumb')) {
    return
  }
  const el = scrollRef.value
  const track = trackEl.value
  const t = thumb.value

  if (!el || !track || !t.scrollable) {
    return
  }
  const rect = track.getBoundingClientRect()
  const yWithinTrack = ev.clientY - rect.top - TRACK_PADDING_PX
  const movableTrack = t.trackHeight - t.heightPx

  if (movableTrack <= 0) {
    return
  }
  // Centre the thumb on the click point — match native scrollbar
  // ergonomics where clicking the track jumps mid-thumb to that y.
  const desiredThumbTop = Math.max(0, Math.min(movableTrack, yWithinTrack - t.heightPx / 2))
  const progress = desiredThumbTop / movableTrack
  const scrollHeight = el.scrollHeight
  const clientHeight = el.clientHeight

  el.scrollTop = progress * (scrollHeight - clientHeight)
}

function onTrackEnter(): void {
  hovering.value = true
  visible.value = true
}

function onTrackLeave(): void {
  hovering.value = false
  showThenScheduleHide()
}
</script>

<template>
  <div
    ref="trackEl"
    class="chat-scrollbar-track"
    :class="{ 'is-visible': visible || hovering || dragging, 'is-scrollable': thumb.scrollable }"
    @pointerdown="onTrackPointerDown"
    @pointerenter="onTrackEnter"
    @pointerleave="onTrackLeave"
  >
    <div
      v-if="thumb.scrollable"
      class="chat-scrollbar-thumb"
      :style="{ top: `${thumb.topPx}px`, height: `${thumb.heightPx}px` }"
      @pointerdown="onThumbPointerDown"
      @pointermove="onThumbPointerMove"
      @pointerup="onThumbPointerUp"
      @pointercancel="onThumbPointerUp"
    />
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.chat-scrollbar-track {
  position: absolute;
  top: 0;
  right: 0;
  width: 0.625rem;
  height: 100%;
  z-index: 4;
  opacity: 0;
  transition: opacity 200ms ease;
  pointer-events: none;
}

.chat-scrollbar-track.is-scrollable {
  pointer-events: auto;
}

.chat-scrollbar-track.is-visible {
  opacity: 1;
}

.chat-scrollbar-thumb {
  position: absolute;
  left: 0.125rem;
  right: 0.125rem;
  border-radius: 9999px;
  background-color: var(--theme-fg-dim);
  opacity: 0.4;
  cursor: pointer;
  transition: opacity 120ms ease;
  touch-action: none;
}

.chat-scrollbar-thumb:hover {
  opacity: 0.7;
}

.chat-scrollbar-thumb:active {
  opacity: 0.85;
}
</style>
