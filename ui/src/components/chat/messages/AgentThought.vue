<script setup lang="ts">
defineProps<{
  text: string
}>()
</script>

<template>
  <article class="agent-thought" data-testid="agent-thought">
    <header class="agent-thought-header">
      <span class="agent-thought-dot" aria-hidden="true" />
      <span class="agent-thought-label">thinking</span>
    </header>
    <p class="agent-thought-body">{{ text }}</p>
  </article>
</template>

<style scoped>
@reference "../../../assets/styles.css";

.agent-thought {
  @apply flex flex-col gap-1 border border-dashed px-3 py-2 text-[0.85rem] italic;
  background-color: var(--theme-surface-card-assistant);
  border-color: var(--theme-border-soft);
  color: var(--theme-fg-dim);
}

.agent-thought-header {
  @apply flex items-center gap-2 text-[0.7rem] uppercase not-italic tracking-wider;
  color: var(--theme-state-awaiting);
}

.agent-thought-dot {
  @apply h-2 w-2 rounded-full;
  background-color: var(--theme-state-awaiting);
}

.agent-thought-label {
  @apply font-bold;
}

.agent-thought-body {
  @apply m-0 whitespace-pre-wrap leading-snug;
  font-family: var(--theme-font-family);
}
</style>
