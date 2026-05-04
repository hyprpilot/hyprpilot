<script setup lang="ts">
import { faChevronDown, faChevronRight } from '@fortawesome/free-solid-svg-icons'
import { ref } from 'vue'

import { MarkdownBody, type ToolField } from '@components'

/**
 * Shared spec-sheet body — used by `ToolPill` (when expanded), the
 * permission row, and the permission modal. Three independent
 * sections:
 *
 *  1. Description — markdown body via `<MarkdownBody>`. ALWAYS
 *     markdown by convention (D7); formatters only set this when the
 *     source is markdown-shaped. MarkdownBody owns the fenced-block
 *     chrome (collapse + copy) so the captain gets working code blocks
 *     without per-consumer wiring.
 *  2. Fields — structured key/value rows (MCP arg dumps, JSON args).
 *  3. Output — preformatted plain text (stdout / diff / file content)
 *     in a collapsible mono pre block.
 *
 * The container itself doesn't cap height — the consumer wraps in
 * a scrollable region. The output `<pre>` caps its own max-height
 * so a 10k-line stream doesn't push every other section off-screen.
 */
defineProps<{
  description?: string
  output?: string
  fields?: ToolField[]
}>()

const outputExpanded = ref(true)

function toggleOutput(): void {
  outputExpanded.value = !outputExpanded.value
}
</script>

<template>
  <div class="spec-sheet">
    <MarkdownBody v-if="description" :source="description" class="spec-sheet-description" />

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

/* Tighter prose tuning on top of MarkdownBody — spec sheets read in
 * a denser surface than transcript bodies. Code-block chrome stays
 * MarkdownBody's. */
.spec-sheet :deep(.spec-sheet-description) {
  @apply text-[0.7rem] leading-relaxed;
  color: var(--theme-fg);
  font-family: var(--theme-font-sans);
  overflow-wrap: anywhere;
}

.spec-sheet :deep(.spec-sheet-description p) {
  @apply my-1;
}

.spec-sheet :deep(.spec-sheet-description p:first-child) {
  @apply mt-0;
}

.spec-sheet :deep(.spec-sheet-description p:last-child) {
  @apply mb-0;
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
  color: var(--theme-fg-subtle);
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
  color: var(--theme-fg-subtle);
  letter-spacing: 0.6px;
}

.spec-sheet-output-body {
  @apply m-0 text-[0.62rem] leading-snug;
  padding: 6px 8px;
  color: var(--theme-fg-subtle);
  white-space: pre-wrap;
  overflow-x: auto;
  max-height: 280px;
  overflow-y: auto;
}
</style>
