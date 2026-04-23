<script setup lang="ts">
/**
 * Colored-left-border body wrapper for a user message. Port of D5's
 * `D5UserBody`. Stateless.
 */
</script>

<template>
  <div class="user-body">
    <slot />
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.user-body {
  @apply border-l-[3px] px-3 py-2 text-[0.9rem] leading-snug;
  color: var(--theme-fg);
  background-color: var(--theme-surface);
  border-color: var(--theme-accent-user);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  min-width: 0;
  font-family: var(--theme-font-sans);
}
</style>
