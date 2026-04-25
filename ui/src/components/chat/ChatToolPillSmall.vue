<script setup lang="ts">
import { computed } from 'vue'

import { ToolState, iconForToolKind, toolStateTone, type ToolChipItem } from '../types'

/**
 * Small-tool chip. Rendered inside the 2-col grid that ToolChips
 * builds from consecutive small-tool items. Port of D5's
 * `D5SmallToolPill`.
 *
 * Border-left + label colour follow the per-state palette (running →
 * streaming / done → dim / failed → err / awaiting → awaiting). The
 * `kind` prop carries the tool-family tint for consumers that want it
 * but is not load-bearing on the visual — the JSX keeps colouring
 * single-state per chip, not dual kind/state.
 */
const props = defineProps<{
  item: ToolChipItem
}>()

const stateTone = computed(() => toolStateTone(props.item.state))

const kindIcon = computed(() => iconForToolKind(props.item.kind))
</script>

<template>
  <span class="tool-pill-small" :data-state="item.state" :style="{ '--tone': stateTone }">
    <span v-if="item.state === ToolState.Running" class="tool-pill-small-dot" aria-hidden="true" />
    <FaIcon :icon="kindIcon" class="tool-pill-small-kind" aria-hidden="true" />
    <span class="tool-pill-small-label">{{ item.label }}</span>
    <span v-if="item.arg" class="tool-pill-small-arg">{{ item.arg }}</span>
    <span v-if="item.stat" class="tool-pill-small-stat">· {{ item.stat }}</span>
  </span>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tool-pill-small {
  @apply inline-flex items-center gap-[6px] border-l-[2px] px-2 py-[2px] text-[0.62rem] leading-tight;
  font-family: var(--theme-font-mono);
  border-color: var(--tone);
  background-color: var(--theme-surface);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  min-width: 0;
  max-width: 100%;
}

.tool-pill-small-dot {
  @apply inline-block h-[4px] w-[4px] shrink-0 animate-pulse-slow rounded-full;
  background-color: var(--tone);
}

.tool-pill-small-kind {
  @apply shrink-0;
  width: 9px;
  height: 9px;
  color: var(--tone);
}

.tool-pill-small-label {
  @apply shrink-0 font-bold;
  color: var(--tone);
}

.tool-pill-small[data-state='done'] .tool-pill-small-arg {
  color: var(--theme-fg-ink-2);
}

.tool-pill-small-arg {
  @apply truncate;
  color: var(--theme-fg);
  min-width: 0;
}

.tool-pill-small-stat {
  @apply shrink-0 text-[0.56rem];
  color: var(--theme-fg-dim);
}
</style>
