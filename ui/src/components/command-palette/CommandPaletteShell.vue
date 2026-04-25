<script setup lang="ts">
import { computed, useSlots } from 'vue'

/**
 * Floating centered palette dialog. Slots:
 *  - title (optional header strip)
 *  - query (input area)
 *  - body (scrolling row list)
 *  - preview (optional side pane; presence auto-promotes width)
 *  - hints (footer keyboard-hint row)
 *
 * Port of D5's `D5PaletteShell`. No fuzzy, no keyboard — presentational
 * only. K-249 builds behaviour on top.
 */
const props = withDefaults(
  defineProps<{
    width?: 'default' | 'wide'
  }>(),
  { width: 'default' }
)

const slots = useSlots()
const effectiveWidth = computed(() => (slots.preview ? 'wide' : props.width))
</script>

<template>
  <div class="palette-shell" :data-width="effectiveWidth" data-testid="palette-shell">
    <header v-if="$slots.title" class="palette-shell-title">
      <slot name="title" />
    </header>

    <div v-if="$slots.query" class="palette-shell-query">
      <slot name="query" />
    </div>

    <div class="palette-shell-content">
      <div class="palette-shell-body">
        <slot name="body" />
      </div>
      <aside v-if="$slots.preview" class="palette-shell-preview">
        <slot name="preview" />
      </aside>
    </div>

    <footer v-if="$slots.hints" class="palette-shell-hints">
      <slot name="hints" />
    </footer>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.palette-shell {
  @apply flex flex-col border;
  border-color: var(--theme-border);
  background-color: var(--theme-surface);
  color: var(--theme-fg);
  box-shadow: 0 12px 40px color-mix(in srgb, var(--theme-surface-bg) 70%, transparent);
  max-height: 70vh;
  min-height: 0;
  /* Never exceed the overlay viewport — widths below are target sizes;
   * max-width clamps against the anchor width on small monitors. */
  max-width: 95vw;
}

.palette-shell[data-width='default'] {
  width: 38rem;
}

.palette-shell[data-width='wide'] {
  width: 56rem;
}

/* Below 560px the preview pane steals too much of the row list for any
 * row label to stay readable. Hide it — the main list remains functional
 * and the preview comes back at width. */
@media (max-width: 560px) {
  .palette-shell-preview {
    display: none;
  }
}

.palette-shell-title {
  @apply flex items-center gap-2 border-b px-3 py-[6px] text-[0.72rem] uppercase tracking-wider;
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.palette-shell-query {
  @apply flex items-center gap-2 border-b px-3 py-2;
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface-alt);
}

.palette-shell-content {
  @apply flex min-h-0 flex-1;
}

.palette-shell-body {
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
}

.palette-shell-preview {
  @apply flex min-h-0 w-2/5 flex-col overflow-y-auto border-l;
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface-alt);
}

.palette-shell-hints {
  @apply flex items-center justify-center gap-[18px] border-t px-[14px] py-[8px] text-[0.7rem];
  border-color: var(--theme-border);
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}
</style>
