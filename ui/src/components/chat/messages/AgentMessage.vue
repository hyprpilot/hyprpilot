<script setup lang="ts">
import { computed } from 'vue'

import { renderMarkdown } from '@lib'

const props = defineProps<{
  text: string
}>()

const html = computed(() => renderMarkdown(props.text))
</script>

<template>
  <article class="agent-message" data-testid="agent-message">
    <header class="agent-message-header">
      <span class="agent-message-dot" aria-hidden="true" />
      <span class="agent-message-label">agent</span>
    </header>
    <!-- eslint-disable-next-line vue/no-v-html -->
    <div class="agent-message-body prose-like" v-html="html" />
  </article>
</template>

<style scoped>
@reference "../../../assets/styles.css";

.agent-message {
  @apply flex flex-col gap-1 border px-3 py-2 text-[0.9rem];
  background-color: var(--theme-surface-card-assistant);
  border-color: var(--theme-border-soft);
  color: var(--theme-fg);
}

.agent-message-header {
  @apply flex items-center gap-2 text-[0.75rem] uppercase tracking-wider;
  color: var(--theme-accent-assistant);
}

.agent-message-dot {
  @apply h-2 w-2 rounded-full;
  background-color: var(--theme-accent-assistant);
}

.agent-message-label {
  @apply font-bold;
}

.agent-message-body {
  @apply leading-snug;
  font-family: var(--theme-font-family);
}

.prose-like :deep(p) {
  @apply my-1;
}

.prose-like :deep(pre) {
  @apply my-2 overflow-auto border px-2 py-1 text-[0.8rem];
  background-color: var(--theme-surface-compose);
  border-color: var(--theme-border-soft);
}

.prose-like :deep(code) {
  font-family: var(--theme-font-family);
}

.prose-like :deep(a) {
  color: var(--theme-accent);
  text-decoration: underline;
}
</style>
