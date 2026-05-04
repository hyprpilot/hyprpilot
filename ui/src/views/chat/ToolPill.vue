<script setup lang="ts">
import { faChevronDown, faChevronRight } from '@fortawesome/free-solid-svg-icons'
import { computed, ref, watch } from 'vue'

import ToolBody from './ToolBody.vue'
import { ToolState, toolStateTone, type ToolCallView } from '@components'

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
 * Auto-expand: running / awaiting → expanded; pending / done /
 * failed → collapsed. Manual toggle suspends auto-default for that
 * pill so subsequent state transitions don't override.
 */
const props = defineProps<{
  view: ToolCallView
}>()

function autoExpand(state: ToolState): boolean {
  return state === ToolState.Running || state === ToolState.Awaiting
}

const expanded = ref(autoExpand(props.view.state))
let manuallyToggled = false

watch(
  () => props.view.state,
  (next) => {
    if (!manuallyToggled) {
      expanded.value = autoExpand(next)
    }
  }
)

function toggle(): void {
  manuallyToggled = true
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
        <span v-if="view.state === ToolState.Running" class="tool-pill-dot" aria-hidden="true" />
        <FaIcon :icon="view.icon" class="tool-pill-icon" aria-hidden="true" />
      </span>
      <span class="tool-pill-title">{{ view.title }}</span>
      <span v-if="view.stat" class="tool-pill-stat">{{ view.stat }}</span>
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
  border-left: 2px solid var(--tone);
  background-color: var(--theme-surface);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  border-radius: 3px;
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
  column-gap: 8px;
  padding: 3px 8px;
}

.tool-pill[data-expanded='true'] .tool-pill-header {
  border-bottom: 1px solid var(--theme-border);
}

.tool-pill-header[role='button'] {
  cursor: pointer;
}

.tool-pill-icon-cell {
  @apply flex items-center gap-[4px];
  flex-shrink: 0;
}

.tool-pill-dot {
  @apply inline-block h-[4px] w-[4px] shrink-0 animate-pulse rounded-full;
  background-color: var(--tone);
}

.tool-pill-icon {
  width: 11px;
  height: 11px;
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

.tool-pill-stat {
  @apply shrink-0 text-[0.56rem];
  color: var(--theme-fg-dim);
}

.tool-pill-caret {
  @apply shrink-0;
  width: 9px;
  height: 9px;
  color: var(--theme-fg-dim);
}

.tool-pill-body {
  @apply flex flex-col overflow-y-auto;
  padding: 8px 10px;
  max-height: 60vh;
}
</style>
