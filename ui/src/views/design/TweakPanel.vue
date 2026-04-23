<script setup lang="ts">
import { computed, ref, watchEffect } from 'vue'

import { Phase } from '@components'

/**
 * Phase-state previewer. Writes `data-agent-state="<phase>"` on
 * `<html>` so the overlay chrome can react to phase tokens. Fixture-only;
 * do not port into the production shell.
 */
const phases = [Phase.Idle, Phase.Streaming, Phase.Pending, Phase.Awaiting, Phase.Working]
const current = ref<Phase>(Phase.Idle)

watchEffect(() => {
  document.documentElement.dataset.agentState = current.value
})

// circle-notch (spinning) for any "in motion" phase; circle for settled states.
const isMotion = computed(() => current.value === Phase.Streaming || current.value === Phase.Pending || current.value === Phase.Working)
const iconName = computed(() => (isMotion.value ? 'circle-notch' : 'circle'))
</script>

<template>
  <aside class="tweak-panel" aria-label="design tweak panel">
    <span class="tweak-panel-label">phase</span>
    <select v-model="current" class="tweak-panel-select">
      <option v-for="p in phases" :key="p" :value="p">{{ p }}</option>
    </select>
    <FaIcon :icon="['fas', iconName]" :class="['tweak-panel-icon', { 'fa-spin': isMotion }]" :style="{ color: `var(--theme-phase-${current})` }" aria-hidden="true" />
  </aside>
</template>

<style scoped>
@reference '../../assets/styles.css';

.tweak-panel {
  @apply fixed bottom-3 right-3 z-50 flex items-center gap-2 border px-2 py-1 text-[0.72rem];
  font-family: var(--theme-font-mono);
  color: var(--theme-fg-ink-2);
  background-color: var(--theme-surface);
  border-color: var(--theme-border);
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.45);
}

.tweak-panel-label {
  color: var(--theme-fg-dim);
  text-transform: lowercase;
}

.tweak-panel-select {
  @apply border px-1 py-[1px] text-[0.72rem] lowercase;
  font-family: var(--theme-font-mono);
  color: var(--theme-fg);
  background-color: var(--theme-surface-bg);
  border-color: var(--theme-border);
}

.tweak-panel-icon {
  width: 12px;
  height: 12px;
}
</style>
