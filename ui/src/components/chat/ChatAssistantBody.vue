<script setup lang="ts">
/**
 * Colored-left-border body wrapper for an assistant message. Port of
 * D5's `D5AssistantBody`. Stateless.
 */
</script>

<template>
  <div class="assistant-body">
    <slot />
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.assistant-body {
  @apply border-l-[3px] px-3 py-2 text-[0.9rem] leading-snug;
  color: var(--theme-fg);
  background-color: var(--theme-surface);
  border-color: var(--theme-accent-assistant);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  overflow-wrap: anywhere;
  min-width: 0;
  font-family: var(--theme-font-sans);
}
</style>
