<script setup lang="ts">
import { faChevronDown, faChevronRight } from '@fortawesome/free-solid-svg-icons'
import { computed, ref } from 'vue'

import ToolBody from './ToolBody.vue'
import ToolPillStats from './ToolPillStats.vue'
import { toolStateTone, type ToolCallView } from '@components'

/**
 * Tool-call pill — collapsible 3-section row with an expandable body.
 *
 *   [icon] [title] [stat] [▾/▸]
 *
 * The title is composed by the formatter (`bash · npm test`, `read ·
 * src/foo.ts (lines 10..30)`, `playwright · browser navigate`); the
 * pill renders it as one ellipsised string. Tone (border-left + icon
 * color) tracks `view.state` per visual law #3.
 *
 * Tools start collapsed. The state tone is enough to show live / done /
 * failed status; expanding for args, diffs, and output is an explicit
 * captain drill-in.
 */
const props = defineProps<{
  view: ToolCallView
}>()

const expanded = ref(false)

function toggle(): void {
  expanded.value = !expanded.value
}

const stateTone = computed(() => toolStateTone(props.view.state))
const hasBody = computed(() => Boolean(props.view.description) || Boolean(props.view.output) || (props.view.fields !== undefined && props.view.fields.length > 0))
const isInteractive = computed(() => hasBody.value)
</script>

<template>
  <span class="tool-pill" :data-state="view.state" :data-expanded="expanded" :data-kind="view.kind" :style="{ '--tone': stateTone }">
    <span
      class="tool-pill-header"
      :role="isInteractive ? 'button' : undefined"
      :tabindex="isInteractive ? 0 : undefined"
      :aria-expanded="isInteractive ? expanded : undefined"
      @click="isInteractive && toggle()"
      @keydown.enter.prevent="isInteractive && toggle()"
      @keydown.space.prevent="isInteractive && toggle()"
    >
      <span class="tool-pill-icon-cell" :aria-label="view.title">
        <FaIcon :icon="view.icon" class="tool-pill-icon" aria-hidden="true" />
      </span>
      <span class="tool-pill-title">{{ view.title }}</span>
      <ToolPillStats :stats="view.stats" />
      <FaIcon v-if="hasBody" :icon="expanded ? faChevronDown : faChevronRight" class="tool-pill-caret" aria-hidden="true" />
    </span>

    <div v-if="expanded && hasBody" class="tool-pill-body">
      <ToolBody :view="view" />
    </div>
  </span>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tool-pill {
  @apply flex flex-col text-[0.62rem] leading-tight;
  font-family: var(--theme-font-mono);
  border-left: 0.125rem solid var(--tone);
  background-color: var(--theme-surface);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  border-radius: 0.1875rem;
  min-width: 0;
  overflow: hidden;
}

.tool-pill[data-expanded='true'] {
  grid-column: 1 / -1;
  border-color: var(--theme-border-soft);
}

.tool-pill-header {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto auto;
  align-items: center;
  column-gap: 0.5rem;
  padding: 0.1875rem 0.5rem;
}

.tool-pill[data-expanded='true'] .tool-pill-header {
  border-bottom: 1px solid var(--theme-border);
}

.tool-pill-header[role='button'] {
  cursor: pointer;
}

.tool-pill-icon-cell {
  @apply flex items-center gap-[0.25rem];
  flex-shrink: 0;
}

.tool-pill-icon {
  width: 0.6875rem;
  height: 0.6875rem;
  color: var(--tone);
  flex-shrink: 0;
}

.tool-pill-title {
  color: var(--theme-fg);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

.tool-pill[data-state='done'] .tool-pill-title {
  color: var(--theme-fg-subtle);
}

.tool-pill-caret {
  @apply shrink-0;
  width: 0.5625rem;
  height: 0.5625rem;
  color: var(--theme-fg-dim);
}

.tool-pill-body {
  @apply flex flex-col overflow-y-auto;
  padding: 0.5rem 0.625rem;
  max-height: 60vh;
}
</style>
