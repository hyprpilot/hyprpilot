<script setup lang="ts">
/**
 * Centered loading affordance — spinner + optional status slot.
 *
 * Three placement modes:
 *
 *  - `fullscreen` — fixed-positioned z-index 100 cover, used during
 *    initial boot before `applyTheme` resolves. Paints over the
 *    Overlay shell so no flash of unstyled content leaks.
 *  - `scoped` — absolute-positioned cover over the nearest
 *    positioned ancestor. Used when a sub-area (chat transcript,
 *    palette body) is loading but the surrounding chrome stays
 *    operational so the user can still navigate / cancel / open
 *    the palette.
 *  - `inline` — block-level, takes whatever vertical space the
 *    parent gives it. Used inside the palette body so the
 *    "loading sessions…" affordance replaces the empty list rather
 *    than sitting on top of it.
 *
 * The optional `status` prop renders as a small pill alongside the
 * spinner — pass a one-line description of the current step
 * ("loading theme", "fetching sessions", "replaying transcript") so
 * the user can follow what's happening rather than staring at an
 * inscrutable spinner. The default slot wins over `status` when both
 * are present, allowing callers to compose richer status content
 * (e.g. a progress count, a kbd hint).
 */
import { faCircleNotch } from '@fortawesome/free-solid-svg-icons'
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
    /** Cover placement. Defaults to `inline`. */
    mode?: 'fullscreen' | 'scoped' | 'inline'
    /** One-line status, rendered as a pill next to the spinner. */
    status?: string
  }>(),
  {
    mode: 'inline',
    status: undefined
  }
)

const wrapperClass = computed(() => `loading loading-${props.mode}`)
</script>

<template>
  <div :class="wrapperClass" role="status" :aria-live="mode === 'fullscreen' ? 'polite' : undefined">
    <div class="loading-inner">
      <FaIcon :icon="faCircleNotch" class="loading-spinner animate-spin" aria-hidden="true" />
      <div v-if="$slots.default || status" class="loading-status">
        <slot>{{ status }}</slot>
      </div>
    </div>
  </div>
</template>

<style scoped>
@reference '../assets/styles.css';

/* Shared centered layout. The `inline` variant flexes inside its
 * parent flow; the cover variants pin to a positioning context. */
.loading {
  @apply flex items-center justify-center;
  font-family: var(--theme-font-mono);
}

.loading-fullscreen {
  position: fixed;
  inset: 0;
  z-index: 100;
  /* Use `--theme-window` (the daemon-resolved overlay backdrop):
   * unambiguous since `surface.default` and `surface.bg` collide on
   * `--theme-surface` after the cssVarName "default/bg drop" rule,
   * leaving `--theme-surface-bg` itself unset. */
  background-color: var(--theme-window);
}

.loading-scoped {
  position: absolute;
  inset: 0;
  z-index: 50;
  /* Fully opaque cover over the parent's positioning context — bleed-
   * through reads as "is this loading or live?" ambiguity. We paint
   * over the parent box completely so partial state never shows. */
  background-color: var(--theme-window);
}

.loading-inline {
  width: 100%;
  padding: 24px 16px;
}

.loading-inner {
  @apply flex flex-col items-center;
  gap: 10px;
}

.loading-spinner {
  width: 18px;
  height: 18px;
  color: var(--theme-accent);
}

/* Status pill — line2 outline, surface-bg fill, fg-subtle text. Same
 * visual vocabulary as the breadcrumb / kbd pills so the user reads
 * it as an operator-facing tag. */
.loading-status {
  @apply inline-flex items-center text-[0.62rem] uppercase;
  background-color: var(--theme-surface-bg);
  border: 1px solid var(--theme-border-soft);
  color: var(--theme-fg-subtle);
  padding: 3px 8px;
  border-radius: 3px;
  letter-spacing: 0.6px;
  font-weight: 600;
  max-width: 48ch;
  text-overflow: ellipsis;
  overflow: hidden;
  white-space: nowrap;
}
</style>
