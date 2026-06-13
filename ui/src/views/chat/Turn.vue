<script setup lang="ts">
import { computed } from 'vue'

import RoleTag from './RoleTag.vue'
import { Role, StatPill } from '@components'
import type { TurnUsage } from '@composables'

/**
 * turn lane: 2px colored left stripe wraps the whole turn (role tag
 * + optional elapsed + usage chips + body). Stripe color is the
 * visual law #2 — every captain or pilot turn is a vertical lane
 * the eye follows through deep tool nests. Captain (`Role.User`) →
 * green; Pilot (`Role.Assistant`) → red.
 */
const props = withDefaults(
  defineProps<{
    role: Role
    elapsed?: string
    live?: boolean
    /// Latest `acp:usage-update` reading for this turn — context
    /// utilisation + cost. Renders as `120k/200k · $0.74` chips
    /// next to the elapsed pill on assistant turns.
    usage?: TurnUsage
    /// Daemon-minted seq of the first item in this block. Carried as
    /// `data-anchor-seq` on the root so `useScrollAnchor` can identify
    /// the captain's anchor row across re-renders, page evictions, and
    /// instance flips. Stable per block — `block.startedAt` is set
    /// when the block is projected and never mutates afterward.
    anchorSeq?: number
  }>(),
  { live: false }
)

const ROLE_LABELS: Record<Role, string> = {
  [Role.User]: 'captain',
  [Role.Assistant]: 'pilot'
}
const roleLabel = ROLE_LABELS[props.role]

function formatTokenCount(n: number): string {
  if (n >= 1_000_000) {
    return `${(n / 1_000_000).toFixed(1)}M`
  }

  if (n >= 1_000) {
    return `${Math.round(n / 1_000)}k`
  }

  return `${n}`
}
const usageLabel = computed(() => {
  const u = props.usage

  if (!u || u.size === 0) {
    return undefined
  }

  return `${formatTokenCount(u.used)}/${formatTokenCount(u.size)}`
})
const costLabel = computed(() => {
  const c = props.usage?.cost

  if (!c) {
    return undefined
  }
  // Currency-symbol mapping for the common cases; fall back to the
  // ISO code when unrecognised so the captain still sees the value.
  const symbol = c.currency === 'USD' ? '$' : c.currency === 'EUR' ? '€' : c.currency
  // Two decimal places is enough resolution at typical session
  // costs (sub-cent variance is noise).

  return `${symbol}${c.amount.toFixed(2)}`
})
</script>

<template>
  <article class="turn" :data-role="role" :data-live="live" :data-anchor-seq="anchorSeq">
    <header class="turn-header">
      <RoleTag :role="role" :label="roleLabel" />
      <div v-if="role === Role.Assistant" class="turn-stats">
        <StatPill v-if="usageLabel" :label="usageLabel" />
        <StatPill v-if="costLabel" :label="costLabel" />
        <StatPill v-if="elapsed" :label="elapsed" :live="live" />
      </div>
    </header>
    <div class="turn-body">
      <slot />
    </div>
  </article>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* turn lane: 2px stripe, 4px padding-left.
 *
 * Keep every row fully measurable. `useScrollAnchor` depends on real
 * `offsetTop` / `offsetHeight`; row-level `content-visibility` swaps
 * offscreen turns to placeholder geometry and makes bottom-follow
 * jump when the browser realizes rows. */
.turn {
  @apply flex flex-col py-1;
  padding-left: 0.25rem;
  border-left: 0.125rem solid var(--theme-accent-user);
  position: relative;
}

.turn[data-role='assistant'] {
  border-left-color: var(--theme-accent-assistant);
}

.turn-header {
  @apply flex items-center gap-2;
  margin-bottom: 0.25rem;
}

/* Push the stats cluster to the right edge of the header. */
.turn-stats {
  @apply flex items-center;
  gap: 0.25rem;
  margin-left: auto;
}

/* 4px gap between turn body children — chunks, tool chips, stream
 * cards stack tightly. */
.turn-body {
  @apply flex flex-col;
  gap: 0.25rem;
}
</style>
