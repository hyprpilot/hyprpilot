<script setup lang="ts">
import { faChevronDown, faChevronRight } from '@fortawesome/free-solid-svg-icons'
import { ref, watch } from 'vue'

import { log, renderMarkdown } from '@lib'

/**
 * Shared spec-sheet body — used by both `ToolPill` (when expanded)
 * and `PermissionStack` to render the structured fields of a tool
 * call. Three focal sections, each independent:
 *
 *  1. Description (markdown prose) — what the tool is doing.
 *  2. Spec rows (command / flags / detail) — structured args.
 *     `kind` is intentionally NOT a spec row: it's already shown
 *     in the consuming component's title (the tool pill's leading
 *     `[icon] Bash` and the permission panel's `[icon] execute · Bash`
 *     both surface kind), so duplicating it here is noise.
 *  3. Output (mono pre, collapsible header) — terminal stdout /
 *     diff / tool result. A separate focal block with its own
 *     chevron + label since it can be very long and the captain
 *     often wants to skim or hide it independently.
 *
 * The container itself doesn't cap height — the consumer wraps in
 * a scrollable region (`PermissionStack` caps the whole panel at
 * 45vh; `ToolPill`'s expanded body caps at 60vh). The output
 * `<pre>` carries its own max-height so a 10k-line stream doesn't
 * push every other section off-screen.
 */
const props = defineProps<{
  description?: string
  command?: string
  flags?: string[]
  detail?: string
  output?: string
}>()

const descriptionHtml = ref('')
watch(
  () => props.description,
  async (raw) => {
    if (!raw) {
      descriptionHtml.value = ''

      return
    }
    try {
      const out = await renderMarkdown(raw)
      descriptionHtml.value = out.html
    } catch (err) {
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

    <div v-if="command" class="spec-sheet-field">
      <span class="spec-sheet-label">command</span>
      <code class="spec-sheet-command">{{ command }}</code>
    </div>
    <div v-if="flags && flags.length > 0" class="spec-sheet-field">
      <span class="spec-sheet-label">flags</span>
      <div class="spec-sheet-kvs">
        <span v-for="(flag, idx) in flags" :key="idx" class="spec-sheet-kv">
          <span class="spec-sheet-kv-key">{{ flag }}</span>
        </span>
      </div>
    </div>
    <div v-if="detail" class="spec-sheet-field">
      <span class="spec-sheet-label">detail</span>
      <span class="spec-sheet-value">{{ detail }}</span>
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

/* Description — Inter prose with markdown sanitised through the
 * shared `renderMarkdown` pipeline. Sits at the top so the captain
 * reads "what I'm doing" before the structured args. */
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
  grid-template-columns: 72px 1fr;
  column-gap: 10px;
  align-items: baseline;
  min-width: 0;
}

.spec-sheet-label {
  @apply text-[0.6rem] uppercase;
  color: var(--theme-fg-ink-2);
  letter-spacing: 0.6px;
  font-weight: 600;
}

.spec-sheet-value {
  color: var(--theme-fg);
  overflow-wrap: anywhere;
}

.spec-sheet-command {
  display: block;
  background-color: var(--theme-surface-bg);
  border: 1px solid var(--theme-border-soft);
  border-radius: 3px;
  padding: 5px 8px;
  white-space: pre;
  color: var(--theme-fg);
  font-size: 0.66rem;
  overflow-x: auto;
}

.spec-sheet-kvs {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

/* KV chip — yellow-accent flag key + line2 outline. */
.spec-sheet-kv {
  display: inline-flex;
  gap: 6px;
  align-items: baseline;
  padding: 2px 7px;
  background-color: var(--theme-surface-bg);
  border: 1px solid var(--theme-border-soft);
  border-radius: 3px;
  font-size: 0.6rem;
}

.spec-sheet-kv-key {
  color: var(--theme-accent);
  font-weight: 700;
}

/* Output — its own section with collapsible header. Visually
 * distinct from the spec rows above (own border + bg) so the eye
 * lands on the focal "what came back" block. */
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
