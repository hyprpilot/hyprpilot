<script setup lang="ts">
/**
 * Inline breadcrumb chip with an optional color override on the left
 * accent bar (the "counts" column in the header). Port of D5's `BCPill`.
 */
withDefaults(
  defineProps<{
    color?: string
    label?: string
    count?: number
  }>(),
  { color: 'var(--theme-border)' }
)
</script>

<template>
  <span class="breadcrumb-pill" :style="{ '--accent': color }">
    <span v-if="label" class="breadcrumb-pill-label">{{ label }}</span>
    <span v-if="count !== undefined" class="breadcrumb-pill-count">{{ count }}</span>
    <slot />
  </span>
</template>

<style scoped>
@reference '../assets/styles.css';

.breadcrumb-pill {
  @apply inline-flex shrink-0 items-center gap-1 border-l-[3px] px-2 py-[2px] text-[0.72rem] leading-tight;
  font-family: var(--theme-font-mono);
  border-color: var(--accent);
  color: var(--theme-fg-ink-2);
  background-color: var(--theme-surface-alt);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
}

.breadcrumb-pill-label {
  color: var(--theme-fg-dim);
  text-transform: lowercase;
}

.breadcrumb-pill-count {
  @apply font-bold;
  color: var(--theme-fg-ink-2);
}
</style>
