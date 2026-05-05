<script setup lang="ts">
import { computed } from 'vue'

import { StatPill } from '@components'
import type { Stat } from '@interfaces/wire/formatted-tool-call'
import { formatDuration } from '@lib'

/**
 * Per-stat-pill renderer. Each `Stat` becomes one or more <StatPill>s
 * laid out in a flex row with 4px gaps. Variants:
 *
 *   - `text`     → one neutral pill carrying the raw string.
 *   - `diff`     → two pills (`+N` ok-toned, `−M` err-toned). Zero
 *                  side hides; both-zero hides the stat entirely.
 *   - `duration` → one neutral pill with `formatDuration(ms)`.
 *   - `matches`  → one neutral pill `N matches` (count=0 hides).
 *
 * Adding a fifth variant: extend the discriminated union in
 * `formatted-tool-call.ts` and add one arm in `renderables` below.
 * StatPill itself stays variant-agnostic.
 */
const props = defineProps<{
  stats: Stat[]
}>()

interface PillView {
  key: string
  label: string
  tone: 'neutral' | 'ok' | 'err'
}

const renderables = computed<PillView[]>(() => {
  const out: PillView[] = []
  // Defensive: a daemon shipping `formatted` without `stats` (older
  // build) would otherwise throw `Cannot read properties of undefined`
  // on `.length` and unmount the entire surrounding tool-chips block.
  const stats = props.stats ?? []

  for (let i = 0; i < stats.length; i++) {
    const stat = stats[i]
    const base = `${i}`

    if (stat.kind === 'text' && stat.value.length > 0) {
      out.push({
        key: `${base}:text`, label: stat.value, tone: 'neutral'
      })
    } else if (stat.kind === 'diff') {
      if (stat.added > 0) {
        out.push({
          key: `${base}:add`, label: `+${stat.added}`, tone: 'ok'
        })
      }

      if (stat.removed > 0) {
        out.push({
          key: `${base}:rem`, label: `−${stat.removed}`, tone: 'err'
        })
      }
    } else if (stat.kind === 'duration') {
      out.push({
        key: `${base}:dur`, label: formatDuration(stat.ms), tone: 'neutral'
      })
    }
  }

  return out
})
</script>

<template>
  <span v-if="renderables.length > 0" class="tool-pill-stats">
    <StatPill v-for="pill in renderables" :key="pill.key" :label="pill.label" :tone="pill.tone" />
  </span>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tool-pill-stats {
  @apply flex shrink-0 items-center;
  gap: 4px;
}
</style>
