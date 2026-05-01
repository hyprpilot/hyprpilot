<script setup lang="ts">
import { ref, watch } from 'vue'

import { log, renderMarkdown } from '@lib'

/**
 * Renders a markdown source through the shared Shiki+DOMPurify
 * pipeline. Default body for `<Modal>` plan / spec / config-error
 * dialogs. Falls back to a `<pre>` block when the pipeline produces
 * empty output (parser hiccup, network-blocked language fetch).
 */
const props = defineProps<{ source: string }>()

const html = ref('')

watch(
  () => props.source,
  async (raw) => {
    if (!raw) {
      html.value = ''

      return
    }
    try {
      const out = await renderMarkdown(raw)
      html.value = out.html
    } catch (err) {
      log.warn('MarkdownBody: render failed', { err: String(err) })
      html.value = ''
    }
  },
  { immediate: true }
)
</script>

<template>
  <!-- eslint-disable-next-line vue/no-v-html -- HTML is sanitised by renderMarkdown -->
  <div v-if="html" class="markdown-body prose" v-html="html" />
  <pre v-else class="markdown-fallback">{{ source }}</pre>
</template>

<style scoped>
@reference '../assets/styles.css';

.markdown-body {
  font-family: var(--theme-font-sans);
  font-size: 0.85rem;
  line-height: 1.5;
  color: var(--theme-fg);
}

.markdown-body :deep(p) {
  margin: 6px 0;
}

.markdown-body :deep(ul),
.markdown-body :deep(ol) {
  margin: 6px 0;
  padding-left: 22px;
  font-size: inherit;
  line-height: inherit;
}

.markdown-body :deep(ul) {
  list-style-type: disc;
}

.markdown-body :deep(ol) {
  list-style-type: decimal;
}

.markdown-body :deep(li) {
  margin: 2px 0;
}

.markdown-body :deep(h1),
.markdown-body :deep(h2),
.markdown-body :deep(h3),
.markdown-body :deep(h4),
.markdown-body :deep(h5),
.markdown-body :deep(h6) {
  margin: 12px 0 6px;
  font-weight: 700;
  color: var(--theme-fg);
  line-height: 1.3;
}

.markdown-body :deep(h1) { font-size: 1.15em; }
.markdown-body :deep(h2) { font-size: 1.05em; }
.markdown-body :deep(h3) { font-size: 1em; }

.markdown-body :deep(code) {
  font-family: var(--theme-font-mono);
  font-size: 0.85em;
  padding: 1px 4px;
  border-radius: 2px;
  background-color: var(--theme-surface-alt);
}

.markdown-body :deep(pre) {
  margin: 8px 0;
  padding: 8px 10px;
  background-color: var(--theme-surface-alt);
  border: 1px solid var(--theme-border-soft);
  border-radius: 3px;
  overflow-x: auto;
}

.markdown-body :deep(pre code) {
  background-color: transparent;
  padding: 0;
}

.markdown-body :deep(blockquote) {
  margin: 6px 0;
  padding-left: 8px;
  border-left: 2px solid var(--theme-border-soft);
  color: var(--theme-fg-dim);
}

.markdown-fallback {
  @apply m-0 overflow-x-auto;
  font-family: var(--theme-font-mono);
  font-size: 0.78rem;
  line-height: 1.4;
  white-space: pre-wrap;
  color: var(--theme-fg);
}
</style>
