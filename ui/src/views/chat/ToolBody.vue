<script setup lang="ts">
import { faChevronDown, faChevronRight } from '@fortawesome/free-solid-svg-icons'
import { computed, ref, watch } from 'vue'

import { MarkdownBody, ToolState, type ToolCallView } from '@components'

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
 *     (collapse + copy) so consumers get working code blocks (Shiki
 *     syntax-highlighted) for free.
 *  2. Fields — structured key/value rows (path, pattern, MCP arg
 *     dumps, JSON args).
 *  3. Output — preformatted plain text (stdout / file content) in a
 *     collapsible mono pre block.
 *
 * Order is description → fields → output to match `## title \n
 * description \n fields-map` — every formatter that emits both a
 * description AND fields treats the description as the human-facing
 * lede and the fields as the structured bag underneath.
 *
 * Returns nothing visible when none of the three are populated; the
 * consumer doesn't need a v-if guard on whether to render.
 */
const props = defineProps<{
  view: ToolCallView
}>()

/// Output expansion mirrors the parent pill's "live show, finalized
/// hide" policy: while the tool is running / awaiting permission,
/// the captain wants to see the streaming stdout in real time. Once
/// the call finalizes (Done / Failed), output collapses so a long
/// completed log doesn't dominate the chat — captain re-expands
/// manually to drill in. Manual toggle pins the captain's choice
/// across subsequent state transitions.
function autoExpandOutput(state: ToolCallView['state']): boolean {
  return state === ToolState.Running || state === ToolState.Awaiting
}

const outputExpanded = ref(autoExpandOutput(props.view.state))
let manuallyToggled = false

watch(
  () => props.view.state,
  (next) => {
    if (!manuallyToggled) {
      outputExpanded.value = autoExpandOutput(next)
    }
  }
)

function toggleOutput(): void {
  manuallyToggled = true
  outputExpanded.value = !outputExpanded.value
}

function normalizeToolBodySection(raw: string | undefined): string {
  if (!raw) {
    return ''
  }

  return raw
    .trim()
    .replace(/^```[^\n]*\n([\s\S]*?)\n?```$/u, '$1')
    .trim()
}

const visibleOutput = computed(() => {
  const output = props.view.output

  if (!output) {
    return undefined
  }

  const normalized = normalizeToolBodySection(output)

  if (!normalized) {
    return undefined
  }

  if (normalized === normalizeToolBodySection(props.view.description)) {
    return undefined
  }

  return output
})

const hasFields = computed(() => Array.isArray(props.view.fields) && props.view.fields.length > 0)
const hasContent = computed(() => Boolean(props.view.description) || hasFields.value || Boolean(visibleOutput.value))
</script>

<template>
  <div v-if="hasContent" class="tool-body">
    <!-- Description renders first — it's the human-facing summary
         the captain reads to understand what the tool call is doing.
         Fields are the structured arg dump beneath; output (when
         present) is the captured stdout/diff at the bottom. -->
    <MarkdownBody v-if="view.description" :source="view.description" class="tool-body-description" />

    <div v-if="hasFields" class="tool-body-fields">
      <div v-for="row in view.fields" :key="row.label" class="tool-body-field">
        <span class="tool-body-label">{{ row.label }}</span>
        <code class="tool-body-code">{{ row.value }}</code>
      </div>
    </div>

    <section v-if="visibleOutput" class="tool-body-output" :data-expanded="outputExpanded">
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
      <!-- Output renders as markdown so fenced code blocks (```bash,
           ```console, ```json, ```diff) get Shiki syntax highlighting
           via the same component the description uses. The agent's
           tool responses are typically narrative + fenced result
           blocks, not raw shell streams; rendering as markdown means
           the captain sees a clean code box instead of literal
           ` ```console ` markup. Real streaming terminal output goes
           through `<TerminalCard>` (terminal_id-bound calls) where
           xterm.js renders ANSI properly — that path is unaffected. -->
      <MarkdownBody v-if="outputExpanded" :source="visibleOutput" class="tool-body-output-body" />
    </section>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tool-body {
  @apply flex flex-col;
  gap: 0.5rem;
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
  gap: 0.375rem;
}

.tool-body-field {
  display: grid;
  grid-template-columns: minmax(0, max-content) 1fr;
  column-gap: 0.75rem;
  align-items: baseline;
  min-width: 0;
}

.tool-body-label {
  @apply text-[0.6rem] uppercase;
  color: var(--theme-fg-subtle);
  letter-spacing: 0.0375rem;
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
  border-radius: 0.1875rem;
  padding: 0.25rem 0.4375rem;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
  font-size: 0.66rem;
}

.tool-body-output {
  border: 1px solid var(--theme-border-soft);
  border-radius: 0.1875rem;
  overflow: hidden;
  background-color: var(--theme-surface-bg);
}

.tool-body-output-header {
  @apply flex items-center gap-2 cursor-pointer;
  padding: 0.25rem 0.5rem;
  background-color: var(--theme-surface);
  user-select: none;
}

.tool-body-output[data-expanded='true'] .tool-body-output-header {
  border-bottom: 1px solid var(--theme-border-soft);
}

.tool-body-output-caret {
  width: 0.5625rem;
  height: 0.5625rem;
  color: var(--theme-fg-dim);
}

.tool-body-output-label {
  @apply text-[0.6rem] uppercase font-bold;
  color: var(--theme-fg-subtle);
  letter-spacing: 0.0375rem;
}

/* MarkdownBody slot — wraps prose paragraphs and fenced code
 * blocks. Same chrome the description path produces, kept narrow
 * enough that long agent narratives stay readable; the inner
 * code blocks Shiki-highlight have their own scrollers. */
.tool-body-output-body {
  padding: 0.375rem 0.5rem;
  max-height: 24rem;
  overflow-y: auto;
}

.tool-body-output-body :deep(p) {
  @apply my-1 text-[0.7rem] leading-relaxed;
  color: var(--theme-fg-subtle);
  font-family: var(--theme-font-sans);
  overflow-wrap: anywhere;
}

.tool-body-output-body :deep(p:first-child) {
  @apply mt-0;
}

.tool-body-output-body :deep(p:last-child) {
  @apply mb-0;
}
</style>
