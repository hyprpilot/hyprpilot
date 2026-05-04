<script setup lang="ts">
import { computed } from 'vue'

import { ToolState, toolStateTone, type ToolCallView } from '@components'

/**
 * Full-bleed tool row — the wider sibling to `ToolPill`. Used for
 * tools the chat surface wants to surface prominently. Three-section
 * layout matching `ToolPill`: `[icon] [title] [stat]`.
 */
const props = defineProps<{
  view: ToolCallView
}>()

const stateTone = computed(() => toolStateTone(props.view.state))
</script>

<template>
  <div class="tool-details" :data-state="view.state" :data-type="view.type" :style="{ '--tone': stateTone }">
    <span v-if="view.state === ToolState.Running" class="tool-details-dot" aria-hidden="true" />
    <FaIcon :icon="view.icon" class="tool-details-kind" aria-hidden="true" />
    <span class="tool-details-title">{{ view.title }}</span>
    <span v-if="view.stat" class="tool-details-stat">{{ view.stat }}</span>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tool-details {
  @apply flex w-full flex-wrap items-center gap-2 border-l-[3px] px-[10px] py-[4px] text-[0.62rem] leading-tight;
  font-family: var(--theme-font-mono);
  border-color: var(--tone);
  color: var(--theme-fg-subtle);
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

.tool-details[data-state='done'] .tool-details-title {
  color: var(--theme-fg-subtle);
}

.tool-details-title {
  @apply flex-1 truncate;
  color: var(--theme-fg);
  min-width: 0;
}

.tool-details-stat {
  @apply shrink-0 text-[0.56rem];
  color: var(--tone);
}
</style>
