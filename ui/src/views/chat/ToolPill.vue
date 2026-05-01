<script setup lang="ts">
import { faChevronDown, faChevronRight } from '@fortawesome/free-solid-svg-icons'
import { computed, ref, watch } from 'vue'

import ToolSpecSheet from './ToolSpecSheet.vue'
import { iconForToolKind, ToolState, toolStateTone, type ToolChipItem } from '@components'

/**
 * Tool-call pill — collapsible row with a 4-section header:
 *
 *   [name (colored, with running pulse)] · [arg · detail …] · [stat] · [▾/▸]
 *
 * Header layout: `auto minmax(0, 1fr) auto auto` CSS grid. `info`
 * ellipsizes; the rest are `flex-shrink: 0` so they never wrap.
 *
 * Tone (border-left + name color) tracks the tool state per visual
 * law #3 — running/awaiting orange, done green, failed red, queued
 * gray.
 *
 * Expansion default tracks state: running / awaiting → expanded
 * (the live work is the foreground concern), pending / done /
 * failed → collapsed. The user can click the header to toggle
 * either direction; once they've toggled manually the auto-default
 * is suspended for that pill so subsequent state transitions don't
 * override the explicit choice.
 *
 * When expanded the pill spans both columns of the parent
 * `ToolChips` grid via `grid-column: 1 / -1` and renders a body
 * section underneath the header — full args + optional `output`
 * payload (terminal stdout, diff, etc.) in a `<pre>` block.
 */
const props = defineProps<{
  item: ToolChipItem
}>()

function autoExpand(state: ToolState): boolean {
  return state === ToolState.Running || state === ToolState.Awaiting
}

const expanded = ref(autoExpand(props.item.state))
let manuallyToggled = false

watch(
  () => props.item.state,
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

const stateTone = computed(() => toolStateTone(props.item.state))
const info = computed(() => [props.item.arg, props.item.detail].filter(Boolean).join(' · '))
const hasBody = computed(
  () =>
    Boolean(props.item.output) || Boolean(props.item.description) || Boolean(props.item.arg)
)
const isInteractive = computed(() => hasBody.value)
const kindIcon = computed(() => iconForToolKind(props.item.kind))
const headerLabel = computed(() => {
  if (props.item.title) {
    return `${props.item.label} · ${props.item.title}`
  }

  return props.item.label
})
</script>

<template>
  <span
    class="tool-pill"
    :data-state="item.state"
    :data-expanded="expanded"
    :style="{ '--tone': stateTone }"
  >
    <span
      class="tool-pill-header"
      :role="isInteractive ? 'button' : undefined"
      :tabindex="isInteractive ? 0 : undefined"
      :aria-expanded="isInteractive ? expanded : undefined"
      @click="isInteractive && toggle()"
      @keydown.enter.prevent="isInteractive && toggle()"
      @keydown.space.prevent="isInteractive && toggle()"
    >
      <span class="tool-pill-name" :aria-label="headerLabel">
        <span v-if="item.state === ToolState.Running" class="tool-pill-dot" aria-hidden="true" />
        <FaIcon :icon="kindIcon" class="tool-pill-icon" aria-hidden="true" />
        <span class="tool-pill-label">
          <span class="tool-pill-kind">{{ item.label }}</span>
          <template v-if="item.title">
            <span class="tool-pill-sep" aria-hidden="true">·</span>
            <span class="tool-pill-title">{{ item.title }}</span>
          </template>
        </span>
      </span>
      <span class="tool-pill-info">{{ info }}</span>
      <span v-if="item.stat" class="tool-pill-stat">{{ item.stat }}</span>
      <FaIcon
        v-if="hasBody"
        :icon="expanded ? faChevronDown : faChevronRight"
        class="tool-pill-caret"
        aria-hidden="true"
      />
    </span>

    <div v-if="expanded && hasBody" class="tool-pill-body">
      <ToolSpecSheet
        :description="item.description"
        :command="item.arg"
        :detail="item.detail"
        :output="item.output"
      />
    </div>
  </span>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* Compact (collapsed) pill — single-row grid header. Expanded pills
 * stretch across both columns of the parent `ToolChips` grid via
 * `grid-column: 1 / -1` so the body has room to lay out. */
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

.tool-pill-name {
  @apply flex shrink-0 items-center gap-[5px];
}

.tool-pill-dot {
  @apply inline-block h-[4px] w-[4px] shrink-0 animate-pulse rounded-full;
  background-color: var(--tone);
}

.tool-pill-icon {
  width: 11px;
  height: 11px;
  color: var(--tone);
}

.tool-pill-label {
  @apply inline-flex shrink-0 items-baseline gap-1;
  min-width: 0;
}

.tool-pill-kind {
  @apply font-bold;
  color: var(--tone);
}

.tool-pill-sep {
  color: var(--theme-fg-faint);
}

.tool-pill-title {
  color: var(--theme-fg-ink-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 24ch;
}

.tool-pill-info {
  color: var(--theme-fg);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

.tool-pill[data-state='done'] .tool-pill-info {
  color: var(--theme-fg-ink-2);
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

/* Expanded body — wraps `ToolSpecSheet` (which carries its own
 * vocabulary). The container caps height + scrolls so a long
 * description + output doesn't push the rest of the transcript out
 * of view. */
.tool-pill-body {
  @apply flex flex-col overflow-y-auto;
  padding: 8px 10px;
  max-height: 60vh;
}
</style>
