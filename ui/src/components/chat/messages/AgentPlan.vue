<script setup lang="ts">
import type { PlanEntry } from '@composables'

defineProps<{
  entries: PlanEntry[]
}>()
</script>

<template>
  <article class="agent-plan" data-testid="agent-plan">
    <header class="agent-plan-header">
      <span class="agent-plan-dot" aria-hidden="true" />
      <span class="agent-plan-label">plan</span>
    </header>

    <ol class="agent-plan-list">
      <li v-for="(entry, idx) in entries" :key="idx" class="agent-plan-entry" :data-status="entry.status ?? 'pending'">
        <span class="agent-plan-entry-status">{{ entry.status ?? 'pending' }}</span>
        <span class="agent-plan-entry-body">{{ entry.content ?? '' }}</span>
      </li>
    </ol>
  </article>
</template>

<style scoped>
@reference "../../../assets/styles.css";

.agent-plan {
  @apply flex flex-col gap-1 border px-3 py-2 text-[0.85rem];
  background-color: var(--theme-surface-card-assistant);
  border-color: var(--theme-border-soft);
  color: var(--theme-fg);
}

.agent-plan-header {
  @apply flex items-center gap-2 text-[0.7rem] uppercase tracking-wider;
  color: var(--theme-state-awaiting);
}

.agent-plan-dot {
  @apply h-2 w-2 rounded-full;
  background-color: var(--theme-state-awaiting);
}

.agent-plan-label {
  @apply font-bold;
}

.agent-plan-list {
  @apply m-0 flex list-none flex-col gap-1 p-0;
}

.agent-plan-entry {
  @apply flex gap-2 leading-snug;
  font-family: var(--theme-font-family);
  color: var(--theme-fg);
}

.agent-plan-entry[data-status='completed'] {
  color: var(--theme-fg-muted);
  text-decoration: line-through;
}

.agent-plan-entry-status {
  @apply shrink-0 text-[0.7rem] uppercase tracking-wider;
  color: var(--theme-fg-muted);
  min-width: 5rem;
}
</style>
