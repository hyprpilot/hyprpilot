<script setup lang="ts">
import { faChevronLeft, faChevronRight } from '@fortawesome/free-solid-svg-icons'
import { onBeforeUnmount, onMounted, ref } from 'vue'

/**
 * Horizontal-scroll container with optional left/right arrow indicators.
 * Wraps a single row of children (pills, chips, kbd hints) and lets
 * them overflow horizontally instead of wrapping. The arrow buttons
 * appear only when there's content offscreen on that side; on touch
 * the captain can swipe directly.
 *
 * Default slot is the row content. The wrapper itself is the scroll
 * container; consumers don't need to manage scroll state.
 */

withDefaults(
  defineProps<{
    /** Pixels to scroll when an arrow is clicked. */
    step?: number
  }>(),
  { step: 160 }
)

const scrollEl = ref<HTMLElement | undefined>(undefined)
const canScrollLeft = ref(false)
const canScrollRight = ref(false)

function updateArrows(): void {
  const el = scrollEl.value

  if (!el) {
    canScrollLeft.value = false
    canScrollRight.value = false

    return
  }
  canScrollLeft.value = el.scrollLeft > 1
  canScrollRight.value = el.scrollLeft + el.clientWidth < el.scrollWidth - 1
}

function scrollBy(delta: number): void {
  scrollEl.value?.scrollBy({ left: delta, behavior: 'smooth' })
}

let resizeObserver: ResizeObserver | undefined

onMounted(() => {
  updateArrows()

  if (typeof ResizeObserver !== 'undefined' && scrollEl.value) {
    resizeObserver = new ResizeObserver(() => updateArrows())
    resizeObserver.observe(scrollEl.value)
  }
})

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  resizeObserver = undefined
})
</script>

<template>
  <div class="horizontal-scroller">
    <button v-if="canScrollLeft" type="button" class="horizontal-scroller-arrow horizontal-scroller-arrow-left" aria-label="scroll left" @click="scrollBy(-step)">
      <FaIcon :icon="faChevronLeft" />
    </button>
    <div ref="scrollEl" class="horizontal-scroller-track" @scroll="updateArrows">
      <slot />
    </div>
    <button v-if="canScrollRight" type="button" class="horizontal-scroller-arrow horizontal-scroller-arrow-right" aria-label="scroll right" @click="scrollBy(step)">
      <FaIcon :icon="faChevronRight" />
    </button>
  </div>
</template>

<style scoped>
@reference '../assets/styles.css';

.horizontal-scroller {
  position: relative;
  display: flex;
  align-items: stretch;
  min-width: 0;
  flex: 1 1 auto;
}

.horizontal-scroller-track {
  display: flex;
  align-items: center;
  gap: var(--horizontal-scroller-gap, 0.5rem);
  min-width: 0;
  flex: 1 1 auto;
  overflow-x: auto;
  scrollbar-width: none;
  scroll-behavior: smooth;
  scroll-snap-type: x proximity;
}

.horizontal-scroller-track::-webkit-scrollbar {
  display: none;
}

.horizontal-scroller-track > :slotted(*) {
  scroll-snap-align: start;
}

.horizontal-scroller-arrow {
  @apply inline-flex shrink-0 cursor-pointer items-center justify-center border-0 bg-transparent;
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1.5rem;
  z-index: 1;
  color: var(--theme-fg-dim);
  background: linear-gradient(to right, var(--theme-surface) 60%, transparent);
}

.horizontal-scroller-arrow-right {
  right: 0;
  background: linear-gradient(to left, var(--theme-surface) 60%, transparent);
}

.horizontal-scroller-arrow-left {
  left: 0;
}

.horizontal-scroller-arrow:hover {
  color: var(--theme-fg);
}

@media (pointer: coarse) {
  .horizontal-scroller-arrow {
    width: 1.75rem;
  }
}
</style>
