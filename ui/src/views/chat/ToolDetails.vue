<script setup lang="ts">
import { computed } from 'vue'

import { ToolState, iconForToolKind, toolStateTone, type ToolChipItem } from '@components'

/**
 * Full-bleed tool row. Used for Bash/Write/Edit/Terminal tools that the
 * grouping logic in ToolChips promotes out of the 2-col grid. Port of
 * the wireframe's BigToolRow.
 *
 * Border, label and stat all take the per-state colour; `arg` bright-fg
 * while running/awaiting, `ink-2` once done; `detail` is italic Inter as
 * a soft meta line. `kind` prop unused visually — the JSX ties everything
 * to state, not tool family.
 */
const props = defineProps<{
  item: ToolChipItem
}>()

const stateTone = computed(() => toolStateTone(props.item.state))

const kindIcon = computed(() => iconForToolKind(props.item.kind))
</script>

<template>
  <div class="tool-details" :data-state="item.state" :style="{ '--tone': stateTone }">
    <span v-if="item.state === ToolState.Running" class="tool-details-dot" aria-hidden="true" />
    <FaIcon :icon="kindIcon" class="tool-details-kind" aria-hidden="true" />
    <span class="tool-details-label">{{ item.label }}</span>
    <span v-if="item.arg" class="tool-details-arg">{{ item.arg }}</span>
    <span v-if="item.detail" class="tool-details-detail">{{ item.detail }}</span>
    <span v-if="item.stat" class="tool-details-stat">{{ item.stat }}</span>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tool-details {
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

.tool-details-dot {
  @apply inline-block h-[5px] w-[5px] shrink-0 animate-pulse rounded-full;
  background-color: var(--tone);
}

.tool-details-kind {
  @apply shrink-0;
  width: 10px;
  height: 10px;
  color: var(--tone);
}

.tool-details-label {
  @apply shrink-0 font-bold;
  color: var(--tone);
  min-width: 36px;
}

.tool-details[data-state='done'] .tool-details-arg {
  color: var(--theme-fg-ink-2);
}

.tool-details-arg {
  @apply flex-1 truncate;
  color: var(--theme-fg);
  min-width: 0;
}

.tool-details-stat {
  @apply shrink-0 text-[0.56rem];
  color: var(--tone);
}

.tool-details-detail {
  @apply truncate text-[0.56rem] italic;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-sans);
}
</style>
