<script setup lang="ts">
import { faChevronDown, faChevronRight } from '@fortawesome/free-solid-svg-icons'
import { computed, ref } from 'vue'

import { MarkdownBody, type ToolCallView } from '@components'

/**
 * Shared body for `ToolPill` (when expanded), `PermissionRow`, and
 * the modal-permission surface — every place that renders the inside
 * of a tool-call card. Takes the unified `ToolCallView` so consumers
 * never re-pluck `description` / `output` / `fields` individually.
 *
 * Three independent sections, in order:
 *
 *  1. Description — markdown body via `<MarkdownBody>` (LLM summary,
 *     fenced command/diff blocks). MarkdownBody owns the fence chrome
 *     (collapse + copy) so consumers get working code blocks for free.
 *  2. Fields — structured key/value rows (path, pattern, MCP arg
 *     dumps, JSON args).
 *  3. Output — preformatted plain text (stdout / file content) in a
 *     collapsible mono pre block.
 *
 * Returns nothing visible when none of the three are populated; the
 * consumer doesn't need a v-if guard on whether to render.
 */
const props = defineProps<{
  view: ToolCallView
}>()

const outputExpanded = ref(true)

function toggleOutput(): void {
  outputExpanded.value = !outputExpanded.value
}

const hasFields = computed(() => Array.isArray(props.view.fields) && props.view.fields.length > 0)
const hasContent = computed(() => Boolean(props.view.description) || hasFields.value || Boolean(props.view.output))
</script>

<template>
  <div v-if="hasContent" class="tool-body">
    <!-- Fields render first — small key/value rows let the captain
         scan path / pattern / pid / etc. before the (often large)
         description block. Edit / patch tools rely on this ordering
         so the diff hunk doesn't push the path off-screen. -->
    <div v-if="hasFields" class="tool-body-fields">
      <div v-for="row in view.fields" :key="row.label" class="tool-body-field">
        <span class="tool-body-label">{{ row.label }}</span>
        <code class="tool-body-code">{{ row.value }}</code>
      </div>
    </div>

    <MarkdownBody v-if="view.description" :source="view.description" class="tool-body-description" />

    <section v-if="view.output" class="tool-body-output" :data-expanded="outputExpanded">
      <header
        class="tool-body-output-header"
        role="button"
        tabindex="0"
        :aria-expanded="outputExpanded"
        @click="toggleOutput"
        @keydown.enter.prevent="toggleOutput"
        @keydown.space.prevent="toggleOutput"
      >
        <FaIcon :icon="outputExpanded ? faChevronDown : faChevronRight" class="tool-body-output-caret" aria-hidden="true" />
        <span class="tool-body-output-label">output</span>
      </header>
      <pre v-if="outputExpanded" class="tool-body-output-body">{{ view.output }}</pre>
    </section>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tool-body {
  @apply flex flex-col;
  gap: 8px;
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
  line-height: 1.55;
  color: var(--theme-fg);
  min-width: 0;
}

.tool-body :deep(.tool-body-description) {
  @apply text-[0.7rem] leading-relaxed;
  color: var(--theme-fg);
  font-family: var(--theme-font-sans);
  overflow-wrap: anywhere;
}

.tool-body :deep(.tool-body-description p) {
  @apply my-1;
}

.tool-body :deep(.tool-body-description p:first-child) {
  @apply mt-0;
}

.tool-body :deep(.tool-body-description p:last-child) {
  @apply mb-0;
}

.tool-body-fields {
  @apply flex flex-col;
  gap: 6px;
}

.tool-body-field {
  display: grid;
  grid-template-columns: minmax(0, max-content) 1fr;
  column-gap: 12px;
  align-items: baseline;
  min-width: 0;
}

.tool-body-label {
  @apply text-[0.6rem] uppercase;
  color: var(--theme-fg-subtle);
  letter-spacing: 0.6px;
  font-weight: 600;
  max-width: 25ch;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tool-body-code {
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

.tool-body-output {
  border: 1px solid var(--theme-border-soft);
  border-radius: 3px;
  overflow: hidden;
  background-color: var(--theme-surface-bg);
}

.tool-body-output-header {
  @apply flex items-center gap-2 cursor-pointer;
  padding: 4px 8px;
  background-color: var(--theme-surface);
  user-select: none;
}

.tool-body-output[data-expanded='true'] .tool-body-output-header {
  border-bottom: 1px solid var(--theme-border-soft);
}

.tool-body-output-caret {
  width: 9px;
  height: 9px;
  color: var(--theme-fg-dim);
}

.tool-body-output-label {
  @apply text-[0.6rem] uppercase font-bold;
  color: var(--theme-fg-subtle);
  letter-spacing: 0.6px;
}

.tool-body-output-body {
  @apply m-0 text-[0.62rem] leading-snug;
  padding: 6px 8px;
  color: var(--theme-fg-subtle);
  white-space: pre-wrap;
  overflow-x: auto;
  max-height: 280px;
  overflow-y: auto;
}
</style>
