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
  width: 360px;
  max-height: 240px;
  background-color: var(--theme-surface);
  border: 1px solid var(--theme-border);
  border-radius: 4px;
  box-shadow: 0 4px 12px rgb(0 0 0 / 0.4);
  font-size: 0.78rem;
  color: var(--theme-fg);
}
</style>
