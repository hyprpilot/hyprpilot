<script setup lang="ts">
import { MarkdownBody } from '@components'

/**
 * Documentation panel rendered alongside the completion popover.
 * Shown when the active item has resolved documentation; hidden
 * otherwise. Markdown body via the existing `<MarkdownBody>`
 * (Shiki + DOMPurify pipeline).
 */
defineProps<{
  documentation: string
}>()
</script>

<template>
  <aside class="completion-docs">
    <MarkdownBody :source="documentation" />
  </aside>
</template>

<style scoped>
@reference '../assets/styles.css';

.completion-docs {
  @apply overflow-y-auto p-3;
  width: 22.5rem;
  max-height: 15rem;
  background-color: var(--theme-surface);
  border: 1px solid var(--theme-border);
  border-radius: 0.25rem;
  box-shadow: 0 0.25rem 0.75rem rgb(0 0 0 / 0.4);
  font-size: 0.78rem;
  color: var(--theme-fg);
}
</style>
