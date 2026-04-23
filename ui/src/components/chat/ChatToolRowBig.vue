<script setup lang="ts">
import { computed } from 'vue'

import { ToolState, iconForToolKind, type ToolChipItem } from '../types'

/**
 * Full-bleed tool row. Used for Bash/Write/Edit/Terminal tools that the
 * grouping logic in ToolChips promotes out of the 2-col grid. Port of
 * D5's `D5BigToolRow`.
 *
 * Border, label and stat all take the per-state colour; `arg` bright-fg
 * while running/awaiting, `ink-2` once done; `detail` is italic Inter as
 * a soft meta line. `kind` prop unused visually — the JSX ties everything
 * to state, not tool family.
 */
const props = defineProps<{
  item: ToolChipItem
}>()

const stateTone = computed(() => {
  switch (props.item.state) {
    case ToolState.Running:
      return 'var(--theme-state-stream)'
    case ToolState.Failed:
      return 'var(--theme-status-err)'
    case ToolState.Awaiting:
      return 'var(--theme-state-awaiting)'
    case ToolState.Done:
    default:
      return 'var(--theme-fg-dim)'
  }
})

const kindIcon = computed(() => iconForToolKind(props.item.kind))
</script>

<template>
  <div class="tool-row-big" :data-state="item.state" :style="{ '--tone': stateTone }">
    <span v-if="item.state === ToolState.Running" class="tool-row-big-dot" aria-hidden="true" />
    <FaIcon :icon="kindIcon" class="tool-row-big-kind" aria-hidden="true" />
    <span class="tool-row-big-label">{{ item.label }}</span>
    <span v-if="item.arg" class="tool-row-big-arg">{{ item.arg }}</span>
    <span v-if="item.detail" class="tool-row-big-detail">{{ item.detail }}</span>
    <span v-if="item.stat" class="tool-row-big-stat">{{ item.stat }}</span>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tool-row-big {
  @apply flex w-full flex-wrap items-center gap-2 border-l-[3px] px-[10px] py-[4px] text-[0.62rem] leading-tight;
  font-family: var(--theme-font-mono);
  border-color: var(--tone);
  color: var(--theme-fg-ink-2);
  background-color: var(--theme-surface);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  min-width: 0;
}

.tool-row-big-dot {
  @apply inline-block h-[5px] w-[5px] shrink-0 animate-pulse-slow rounded-full;
  background-color: var(--tone);
}

.tool-row-big-kind {
  @apply shrink-0;
  width: 10px;
  height: 10px;
  color: var(--tone);
}

.tool-row-big-label {
  @apply shrink-0 font-bold;
  color: var(--tone);
  min-width: 36px;
}

.tool-row-big[data-state='done'] .tool-row-big-arg {
  color: var(--theme-fg-ink-2);
}

.tool-row-big-arg {
  @apply flex-1 truncate;
  color: var(--theme-fg);
  min-width: 0;
}

.tool-row-big-stat {
  @apply shrink-0 text-[0.56rem];
  color: var(--tone);
}

.tool-row-big-detail {
  @apply truncate text-[0.56rem] italic;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-sans);
}
</style>
