<script setup lang="ts">
/**
 * Generic chip with a 3-edge border (top/right/bottom), tinted by `color`.
 * The colored 4th edge (left) is drawn by the consumer via the parent row.
 * Port of D5's `Pill`. Stateless.
 */
withDefaults(
  defineProps<{
    color?: string
    mono?: boolean
  }>(),
  { color: 'var(--theme-border)', mono: false }
)
</script>

<template>
  <span class="pill" :class="{ 'is-mono': mono }" :style="{ borderColor: color }">
    <slot />
  </span>
</template>

<style scoped>
@reference '../assets/styles.css';

.pill {
  @apply inline-flex shrink-0 items-center gap-1 border px-2 py-[2px] text-[0.72rem] leading-tight;
  color: var(--theme-fg-ink-2);
  background-color: var(--theme-surface-alt);
}

.pill.is-mono {
  font-family: var(--theme-font-mono);
}
</style>
