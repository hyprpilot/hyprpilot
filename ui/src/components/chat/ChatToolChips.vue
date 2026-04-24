<script setup lang="ts">
import { computed } from 'vue'

import ToolPillSmall from './ChatToolPillSmall.vue'
import ToolRowBig from './ChatToolRowBig.vue'
import { ToolKind, type ToolChipItem } from '../types'

/**
 * Two rendering modes:
 *
 *   - `grouped` (per-turn cluster): every tool call for the current
 *     turn renders uniformly as a small pill in a masonry-style grid,
 *     regardless of kind. Used between thoughts/plan and the
 *     assistant reply body.
 *   - inline (default): consecutive small-tool items pack into a
 *     flex-wrap row; big tools (Bash/Write/Terminal) get promoted to
 *     full-bleed rows. Dispatch is keyed on `item.kind` (ToolKind
 *     enum) — not on the chip label — so the formatter registry is
 *     the single source of promotion. Edit-family tools carry
 *     `ToolKind.Write` so they big-row too.
 */
const BIG_KINDS: readonly ToolKind[] = [ToolKind.Bash, ToolKind.Write, ToolKind.Terminal]

const props = withDefaults(
  defineProps<{
    items: ToolChipItem[]
    grouped?: boolean
  }>(),
  { grouped: false }
)

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
    if (item.kind !== undefined && BIG_KINDS.includes(item.kind)) {
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
  <div v-if="grouped" class="tool-chips-grid" data-testid="tool-chips">
    <ToolPillSmall v-for="(item, i) in items" :key="i" :item="item" />
  </div>
  <div v-else class="tool-chips" data-testid="tool-chips">
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

/* `grid-auto-flow: dense` packs shorter items into earlier-row gaps, */
/* giving the masonry feel at a fraction of the complexity.           */
.tool-chips-grid {
  @apply grid gap-1;
  grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr));
  grid-auto-flow: dense;
}
</style>
