<script setup lang="ts">
import { faChevronDown, faChevronRight } from '@fortawesome/free-solid-svg-icons'
import { ref, watch } from 'vue'

import type { ToolField } from '@components'
import { log, renderMarkdown } from '@lib'

/**
 * Shared spec-sheet body — used by `ToolPill` (when expanded), the
 * permission row, and the permission modal. Three independent
 * sections:
 *
 *  1. Description — markdown body (rendered through `renderMarkdown`).
 *     ALWAYS markdown by convention (D7); formatters only set this
 *     when the source is markdown-shaped.
 *  2. Fields — structured key/value rows (MCP arg dumps, JSON args).
 *  3. Output — preformatted plain text (stdout / diff / file content)
 *     in a collapsible mono pre block.
 *
 * The container itself doesn't cap height — the consumer wraps in
 * a scrollable region. The output `<pre>` caps its own max-height
 * so a 10k-line stream doesn't push every other section off-screen.
 */
const props = defineProps<{
  description?: string
  output?: string
  fields?: ToolField[]
}>()

const descriptionHtml = ref('')

watch(
  () => props.description,
  async(raw) => {
    if (!raw) {
      descriptionHtml.value = ''

      return
    }

    try {
      const out = await renderMarkdown(raw)

      descriptionHtml.value = out.html
    } catch(err) {
      log.warn('spec-sheet: markdown render failed; plain fallback', { err: String(err) })
      descriptionHtml.value = ''
    }
  },
  { immediate: true }
)

const outputExpanded = ref(true)

function toggleOutput(): void {
  outputExpanded.value = !outputExpanded.value
}
</script>

<template>
  <div class="spec-sheet">
    <!-- eslint-disable-next-line vue/no-v-html -- HTML is sanitised by renderMarkdown -->
    <div v-if="descriptionHtml" class="spec-sheet-description prose" v-html="descriptionHtml" />
    <div v-else-if="description" class="spec-sheet-description-plain">{{ description }}</div>

    <div v-for="row in fields ?? []" :key="row.label" class="spec-sheet-field">
      <span class="spec-sheet-label">{{ row.label }}</span>
      <code class="spec-sheet-code">{{ row.value }}</code>
    </div>

    <section v-if="output" class="spec-sheet-output" :data-expanded="outputExpanded">
      <header
        class="spec-sheet-output-header"
        role="button"
        tabindex="0"
        :aria-expanded="outputExpanded"
        @click="toggleOutput"
        @keydown.enter.prevent="toggleOutput"
        @keydown.space.prevent="toggleOutput"
      >
        <FaIcon :icon="outputExpanded ? faChevronDown : faChevronRight" class="spec-sheet-output-caret" aria-hidden="true" />
        <span class="spec-sheet-output-label">output</span>
      </header>
      <pre v-if="outputExpanded" class="spec-sheet-output-body">{{ output }}</pre>
    </section>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.spec-sheet {
  @apply flex flex-col;
  gap: 8px;
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
  line-height: 1.55;
  color: var(--theme-fg);
  min-width: 0;
}

.spec-sheet-description {
  @apply text-[0.7rem] leading-relaxed;
  color: var(--theme-fg);
  font-family: var(--theme-font-sans);
  overflow-wrap: anywhere;
}

.spec-sheet-description :deep(p) {
  @apply my-1;
}

.spec-sheet-description :deep(p:first-child) {
  @apply mt-0;
}

.spec-sheet-description :deep(p:last-child) {
  @apply mb-0;
}

.spec-sheet-description :deep(code) {
  @apply rounded-sm px-1 py-[1px] text-[0.85em];
  font-family: var(--theme-font-mono);
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
}

.spec-sheet-description-plain {
  @apply text-[0.7rem] leading-relaxed;
  color: var(--theme-fg);
  font-family: var(--theme-font-sans);
  white-space: pre-wrap;
}

.spec-sheet-field {
  display: grid;
  grid-template-columns: minmax(0, max-content) 1fr;
  column-gap: 12px;
  align-items: baseline;
  min-width: 0;
}

.spec-sheet-label {
  @apply text-[0.6rem] uppercase;
  color: var(--theme-fg-ink-2);
  letter-spacing: 0.6px;
  font-weight: 600;
  max-width: 25ch;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.spec-sheet-value {
  color: var(--theme-fg);
  overflow-wrap: anywhere;
}

/* Field-value code block — every parsed primitive (command, path,
 * query, url, …) renders here. Background + border + padding mirror
 * the focal `command` block on the old spec sheet so the captain
 * always reads field values in the same monospace surface. */
.spec-sheet-code {
  display: block;
  background-color: var(--theme-surface-bg);
  border: 1px solid var(--theme-border-soft);
  border-radius: 3px;
  padding: 4px 7px;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
  font-size: 0.66rem;
}

.spec-sheet-output {
  border: 1px solid var(--theme-border-soft);
  border-radius: 3px;
  overflow: hidden;
  background-color: var(--theme-surface-bg);
}

.spec-sheet-output-header {
  @apply flex items-center gap-2 cursor-pointer;
  padding: 4px 8px;
  background-color: var(--theme-surface);
  user-select: none;
}

.spec-sheet-output[data-expanded='true'] .spec-sheet-output-header {
  border-bottom: 1px solid var(--theme-border-soft);
}

.spec-sheet-output-caret {
  width: 9px;
  height: 9px;
  color: var(--theme-fg-dim);
}

.spec-sheet-output-label {
  @apply text-[0.6rem] uppercase font-bold;
  color: var(--theme-fg-ink-2);
  letter-spacing: 0.6px;
}

.spec-sheet-output-body {
  @apply m-0 text-[0.62rem] leading-snug;
  padding: 6px 8px;
  color: var(--theme-fg-ink-2);
  white-space: pre-wrap;
  overflow-x: auto;
  max-height: 280px;
  overflow-y: auto;
}
</style>
