<script setup lang="ts">
import { computed } from 'vue'

import { Phase, phaseToCssSuffix, type SessionRowData } from '@components'

/**
 * Single row of the idle-state live-sessions grid. Dot · title · cwd ·
 * adapter · doing. The left border + dot are phase-colored. Port of the
 * inline grid row in the idle screen.
 */
const props = defineProps<{
  session: SessionRowData
}>()

const emit = defineEmits<{
  focus: [id: string]
}>()

const phaseColor = computed(() => `var(--theme-state-${phaseToCssSuffix(props.session.phase)})`)

interface PhaseIcon {
  pack: 'fas' | 'far'
  name: string
  spin: boolean
}

const phaseIcon = computed<PhaseIcon>(() => {
  switch (props.session.phase) {
    case Phase.Streaming:
    case Phase.Working:
      return { pack: 'fas', name: 'circle', spin: false }
    case Phase.Awaiting:
      return { pack: 'fas', name: 'circle-half-stroke', spin: false }
    case Phase.Pending:
    case Phase.Idle:
    default:
      return { pack: 'far', name: 'circle', spin: false }
  }
})
</script>

<template>
  <button type="button" class="session-row" :data-phase="session.phase" :style="{ '--tone': phaseColor }" @click="emit('focus', session.id)">
    <span class="session-row-dot" aria-hidden="true">
      <FaIcon :icon="[phaseIcon.pack, phaseIcon.name]" :class="{ 'fa-spin': phaseIcon.spin }" />
    </span>
    <span class="session-row-title">{{ session.title }}</span>
    <span class="session-row-cwd">{{ session.cwd }}</span>
    <span class="session-row-adapter">{{ session.adapter }}</span>
    <span class="session-row-doing">{{ session.doing }}</span>
  </button>
</template>

<style scoped>
@reference '../../assets/styles.css';

.session-row {
  @apply grid w-full items-center gap-2 border-l-[3px] px-3 py-[6px] text-[0.66rem];
  grid-template-columns: 16px 220px 170px 90px 110px;
  font-family: var(--theme-font-mono);
  border-color: var(--tone);
  background-color: var(--theme-surface);
  color: var(--theme-fg-ink-2);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  cursor: pointer;
}

.session-row:hover {
  background-color: var(--theme-surface-alt);
}

.session-row[data-phase='streaming'] .session-row-dot {
  @apply animate-pulse;
}

.session-row-dot {
  @apply inline-flex items-center justify-center text-[0.5rem];
  color: var(--tone);
}

.session-row-title {
  @apply truncate px-[6px] text-center;
  color: var(--theme-fg);
}

.session-row-cwd {
  @apply truncate text-center;
  color: var(--theme-fg-ink-2);
}

.session-row-adapter {
  @apply text-center;
  color: var(--theme-fg-ink-2);
}

.session-row-doing {
  @apply truncate text-center;
  color: var(--tone);
}
</style>
