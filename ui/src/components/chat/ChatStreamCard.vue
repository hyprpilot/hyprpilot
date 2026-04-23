<script setup lang="ts">
import { computed, useSlots } from 'vue'

import { PlanStatus, StreamKind, type PlanItem } from '../types'

/**
 * Thinking / planning stream card.
 *
 * Render modes:
 *  (a) active && items.length > 0 → checklist (square-check completed,
 *      circle-half-stroke in-progress, square pending).
 *  (b) active && default-slot content (no items) → <pre> block rendering
 *      the slot.
 *  (c) !active → single-line summary below the header.
 *
 * Port of D5's `StreamCard`. Stateless.
 */
const props = defineProps<{
  kind: StreamKind
  active: boolean
  label: string
  elapsed?: string
  summary?: string
  items?: PlanItem[]
}>()

const slots = useSlots()

// planning → agent (purple); thinking → think (muted slate).
const tone = computed(() => (props.kind === StreamKind.Planning ? 'var(--theme-kind-agent)' : 'var(--theme-kind-think)'))

interface StatusIcon {
  pack: 'fas' | 'far'
  name: string
}

function iconFor(status: PlanStatus): StatusIcon {
  switch (status) {
    case PlanStatus.Completed:
      return { pack: 'fas', name: 'square-check' }
    case PlanStatus.InProgress:
      return { pack: 'fas', name: 'circle-half-stroke' }
    case PlanStatus.Pending:
    default:
      return { pack: 'far', name: 'square' }
  }
}

const hasItems = computed(() => (props.items?.length ?? 0) > 0)
const hasSlot = computed(() => Boolean(slots.default))
</script>

<template>
  <article class="stream-card" :data-kind="kind" :data-active="active" :style="{ '--tone': tone }">
    <header class="stream-card-header">
      <span class="stream-card-dot" aria-hidden="true" />
      <span class="stream-card-label">{{ label }}</span>
      <span v-if="elapsed" class="stream-card-elapsed">{{ elapsed }}</span>
    </header>

    <ul v-if="active && hasItems" class="stream-card-list">
      <li v-for="(item, idx) in items" :key="idx" class="stream-card-item" :data-status="item.status">
        <span class="stream-card-glyph" aria-hidden="true">
          <FaIcon :icon="[iconFor(item.status).pack, iconFor(item.status).name]" />
        </span>
        <span class="stream-card-text">{{ item.text }}</span>
      </li>
    </ul>
    <pre v-else-if="active && hasSlot" class="stream-card-pre"><slot /></pre>
    <p v-else-if="summary" class="stream-card-summary">{{ summary }}</p>
  </article>
</template>

<style scoped>
@reference '../../assets/styles.css';

.stream-card {
  @apply flex flex-col gap-1 border-l-[3px] px-3 py-2 text-[0.82rem] leading-snug;
  color: var(--theme-fg);
  border-color: var(--tone);
  background-color: var(--theme-surface);
  border-top: 1px solid var(--theme-border-soft);
  border-right: 1px solid var(--theme-border-soft);
  border-bottom: 1px solid var(--theme-border-soft);
  font-family: var(--theme-font-sans);
}

.stream-card[data-active='false'] {
  background-color: transparent;
  border-top-color: var(--theme-border);
  border-right-color: var(--theme-border);
  border-bottom-color: var(--theme-border);
  padding-top: 4px;
  padding-bottom: 4px;
}

.stream-card-header {
  @apply flex items-center gap-2 text-[0.7rem] uppercase tracking-wider;
  color: var(--tone);
  font-family: var(--theme-font-mono);
}

.stream-card[data-active='true'] .stream-card-dot {
  @apply animate-pulse-slow;
}

.stream-card-dot {
  @apply h-[6px] w-[6px] rounded-full;
  background-color: var(--tone);
}

.stream-card-label {
  @apply font-bold;
}

.stream-card-elapsed {
  @apply ml-auto text-[0.68rem] normal-case;
  color: var(--theme-fg-faint);
}

.stream-card-list {
  @apply m-0 flex list-none flex-col gap-1 p-0;
}

.stream-card-item {
  @apply flex items-start gap-2;
  font-family: var(--theme-font-mono);
}

.stream-card-item[data-status='completed'] .stream-card-glyph {
  color: var(--theme-status-ok);
}

.stream-card-item[data-status='completed'] .stream-card-text {
  color: var(--theme-fg-dim);
  text-decoration: line-through;
}

.stream-card-item[data-status='in_progress'] .stream-card-glyph {
  color: var(--theme-state-stream);
}

.stream-card-glyph {
  @apply inline-flex shrink-0 items-center justify-center text-[0.625rem];
  color: var(--theme-fg-dim);
  width: 12px;
  height: 12px;
}

.stream-card-text {
  @apply flex-1;
  color: var(--theme-fg-ink-2);
}

.stream-card-pre {
  @apply m-0 overflow-auto px-2 py-1 text-[0.78rem];
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg-ink-2);
  font-family: var(--theme-font-mono);
  border: 1px solid var(--theme-border-soft);
}

.stream-card-summary {
  @apply m-0;
  color: var(--theme-fg-dim);
}
</style>
