<script setup lang="ts">
import { computed } from 'vue'

import type { ToolCallSnapshot } from '@composables'
import { escapeHtml, renderMarkdown } from '@lib'

const props = defineProps<{
  call: ToolCallSnapshot
}>()

const renderedContent = computed(() =>
  props.call.content
    .map((block) => {
      if (typeof block.text === 'string') {
        return renderMarkdown(block.text)
      }

      return `<pre>${escapeHtml(JSON.stringify(block, null, 2))}</pre>`
    })
    .join('')
)

const statusLabel = computed(() => props.call.status ?? 'pending')
</script>

<template>
  <article class="agent-tool" data-testid="agent-tool-call">
    <header class="agent-tool-header">
      <span class="agent-tool-dot" aria-hidden="true" />
      <span class="agent-tool-label">tool</span>
      <span class="agent-tool-title">{{ call.title ?? call.toolCallId }}</span>
      <span class="agent-tool-status">{{ statusLabel }}</span>
    </header>

    <!-- eslint-disable-next-line vue/no-v-html -->
    <div v-if="call.content.length > 0" class="agent-tool-body" v-html="renderedContent" />
  </article>
</template>

<style scoped>
@reference "../../../assets/styles.css";

.agent-tool {
  @apply flex flex-col gap-1 border px-3 py-2 text-[0.85rem];
  background-color: var(--theme-surface-compose);
  border-color: var(--theme-border-soft);
  color: var(--theme-fg);
}

.agent-tool-header {
  @apply flex items-center gap-2 text-[0.7rem] uppercase tracking-wider;
  color: var(--theme-accent);
}

.agent-tool-dot {
  @apply h-2 w-2 rounded-full;
  background-color: var(--theme-accent);
}

.agent-tool-label {
  @apply font-bold;
}

.agent-tool-title {
  @apply flex-1 truncate normal-case;
  color: var(--theme-fg);
}

.agent-tool-status {
  color: var(--theme-fg-muted);
}

.agent-tool-body {
  @apply text-[0.8rem] leading-snug;
  font-family: var(--theme-font-family);
  color: var(--theme-fg-dim);
}

.agent-tool-body :deep(pre) {
  @apply my-1 overflow-auto px-2 py-1;
  background-color: var(--theme-window);
}
</style>
