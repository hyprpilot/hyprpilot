<script setup lang="ts">
import { computed } from 'vue'

import ToolPillSmall from './ChatToolPillSmall.vue'
import ToolRowBig from './ChatToolRowBig.vue'
import type { ToolChipItem } from '../types'

/**
 * Container that groups consecutive small-tool items into flex-wrap
 * rows and promotes big tools (Bash/Write/Edit/Terminal) to full-bleed
 * rows. Small chips pack left-to-right and wrap to additional lines
 * when the container runs out of room. Port of D5's `D5ToolChips` +
 * `D5SmallToolRow` + `D5BigToolRow` dispatch — the 2-col grid in the
 * JSX is intentionally relaxed to flex-wrap here so narrow anchors
 * don't waste the row on a single chip.
 */
const BIG_TOOLS = ['Bash', 'Write', 'Edit', 'Terminal']

const props = defineProps<{
  items: ToolChipItem[]
}>()

interface SmallGroup {
  kind: 'small'
  items: ToolChipItem[]
}
interface BigGroup {
  kind: 'big'
  item: ToolChipItem
}
type Group = SmallGroup | BigGroup

const groups = computed<Group[]>(() => {
  const result: Group[] = []
  let buffer: ToolChipItem[] = []
  const flush = (): void => {
    if (buffer.length > 0) {
      result.push({ kind: 'small', items: buffer })
      buffer = []
    }
  }
  for (const item of props.items) {
    if (BIG_TOOLS.includes(item.label)) {
      flush()
      result.push({ kind: 'big', item })
    } else {
      buffer.push(item)
    }
  }
  flush()

  return result
})
</script>

<template>
  <div class="tool-chips" data-testid="tool-chips">
    <template v-for="(group, idx) in groups" :key="idx">
      <div v-if="group.kind === 'small'" class="tool-chips-small-row">
        <ToolPillSmall v-for="(item, j) in group.items" :key="j" :item="item" />
      </div>
      <ToolRowBig v-else :item="group.item" />
    </template>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tool-chips {
  @apply flex flex-col gap-1;
}

.tool-chips-small-row {
  @apply flex flex-wrap gap-1;
}
</style>
