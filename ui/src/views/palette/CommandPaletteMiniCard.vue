<script setup lang="ts">
/**
 * Compact sibling-stackable palette. Today used by Sessions preview
 * (nested recent sessions list). Port of the wireframe's MiniPalette.
 */
defineProps<{
  title?: string
}>()
</script>

<template>
  <div class="mini-palette-card" data-testid="mini-palette-card">
    <header v-if="title" class="mini-palette-card-title">{{ title }}</header>
    <div class="mini-palette-card-body">
      <slot />
    </div>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.mini-palette-card {
  @apply flex flex-col border;
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface);
  color: var(--theme-fg);
}

.mini-palette-card-title {
  @apply border-b px-2 py-[4px] text-[0.68rem] uppercase tracking-wider;
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.mini-palette-card-body {
  @apply flex flex-col;
}
</style>
