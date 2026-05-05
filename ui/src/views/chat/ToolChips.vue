<script setup lang="ts">
import { faChevronDown, faChevronRight } from '@fortawesome/free-solid-svg-icons'
import { computed, ref } from 'vue'

import ToolPill from './ToolPill.vue'
import type { ToolCallView } from '@components'
import { formatDuration } from '@lib'

/**
 * ToolChips block — collapsible container holding ALL tool calls
 * for a turn. Header reads "▾ TOOLS · N calls · 9.6s". Body is a
 * 2-col grid of ToolPills; expanded pills span both columns.
 *
 * Block elapsed: caller passes `elapsed`; otherwise we sum every
 * `Stat::Duration { ms }` across `views[].stats` and format
 * compactly via `formatDuration`.
 */
const props = withDefaults(
  defineProps<{
    views: ToolCallView[]
    label?: string
    elapsed?: string
  }>(),
  { label: 'tools' }
)

const expanded = ref(true)

function toggle(): void {
  expanded.value = !expanded.value
}

const computedElapsed = computed(() => {
  if (props.elapsed) {
    return props.elapsed
  }
  const totalMs = props.views.reduce((acc, v) => {
    const sum = (v.stats ?? []).reduce((subAcc, s) => (s.kind === 'duration' ? subAcc + s.ms : subAcc), 0)

    return acc + sum
  }, 0)

  return totalMs > 0 ? formatDuration(totalMs) : ''
})
</script>

<template>
  <section class="tool-chips" :data-expanded="expanded" data-testid="tool-chips">
    <header class="tool-chips-header" role="button" tabindex="0" @click="toggle" @keydown.enter.prevent="toggle" @keydown.space.prevent="toggle">
      <FaIcon :icon="expanded ? faChevronDown : faChevronRight" class="tool-chips-caret" aria-hidden="true" />
      <span class="tool-chips-label">{{ label }}</span>
      <span class="tool-chips-meta">· {{ views.length }} call{{ views.length === 1 ? '' : 's' }}</span>
      <span v-if="computedElapsed" class="tool-chips-meta">· {{ computedElapsed }}</span>
    </header>
    <div v-if="expanded" class="tool-chips-grid">
      <ToolPill v-for="view in views" :key="view.id" :view="view" />
    </div>
  </section>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tool-chips {
  background-color: var(--theme-surface);
  border-left: 3px solid var(--theme-kind-read);
  border-top: 1px solid var(--theme-border-soft);
  border-right: 1px solid var(--theme-border-soft);
  border-bottom: 1px solid var(--theme-border-soft);
  border-radius: 4px;
  padding: 6px 10px;
}

.tool-chips-header {
  @apply flex items-center gap-2 text-[0.62rem] uppercase;
  font-family: var(--theme-font-mono);
  letter-spacing: 0.4px;
}

.tool-chips-caret {
  width: 10px;
  height: 10px;
  color: var(--theme-fg);
}

.tool-chips-label {
  @apply font-bold;
  color: var(--theme-kind-read);
}

.tool-chips-meta {
  color: var(--theme-fg-dim);
  text-transform: none;
  letter-spacing: normal;
}

.tool-chips-grid {
  margin-top: 6px;
  padding-top: 6px;
  border-top: 1px dashed var(--theme-border);
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 3px;
}
</style>
