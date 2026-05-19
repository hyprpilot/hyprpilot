<script setup lang="ts">
/**
 * One row in the idle-screen instances table. Owns the per-instance
 * `useSessionInfo` subscription so live updates (title arriving,
 * mode flip, agent rename) re-render the row without the parent
 * having to thread a Map through props.
 *
 * Click / Enter / Space → emit `pick` so the parent can focus the
 * instance. The actual focus RPC lives in Overlay.vue's handler.
 */
import { computed } from 'vue'

import { useSessionInfo, type InstanceId } from '@composables'

const props = defineProps<{
  instanceId: InstanceId
}>()

const emit = defineEmits<{
  pick: [instanceId: InstanceId]
}>()

const { info } = useSessionInfo(props.instanceId)

const headline = computed<string>(() => {
  const sess = info.value

  return sess.name ?? sess.title ?? sess.profileId ?? sess.agent ?? props.instanceId.slice(0, 8)
})

const cwdLabel = computed<string>(() => info.value.cwd ?? '—')
const agentLabel = computed<string>(() => {
  const parts: string[] = []
  const sess = info.value

  if (sess.agent) {
    parts.push(sess.agent)
  }

  if (sess.model) {
    parts.push(sess.model)
  }

  return parts.length > 0 ? parts.join(' · ') : '—'
})

function onActivate(): void {
  emit('pick', props.instanceId)
}
</script>

<template>
  <div
    class="idle-instances-row"
    role="button"
    tabindex="0"
    :aria-label="`focus instance ${headline}`"
    @click="onActivate"
    @keydown.enter.prevent="onActivate"
    @keydown.space.prevent="onActivate"
  >
    <span class="idle-instances-dot" aria-hidden="true">●</span>
    <span class="idle-instances-cell">{{ headline }}</span>
    <span class="idle-instances-cell idle-instances-cwd">{{ cwdLabel }}</span>
    <span class="idle-instances-cell idle-instances-agent">{{ agentLabel }}</span>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.idle-instances-row {
  display: grid;
  grid-template-columns: 0.875rem minmax(0, 1fr) minmax(0, 10.625rem) minmax(0, 6.875rem);
  column-gap: 0.75rem;
  align-items: center;
  padding: 0.4375rem 0.625rem;
  border-bottom: 1px solid var(--theme-border);
  border-left: 0.125rem solid var(--theme-state-stream);
  background-color: var(--theme-surface);
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
  color: var(--theme-fg);
  cursor: pointer;
  transition: background-color 0.12s ease-out;
}

.idle-instances-row:hover,
.idle-instances-row:focus-visible {
  background-color: var(--theme-surface-alt);
  outline: 0;
}

.idle-instances-dot {
  color: var(--theme-state-stream);
}

.idle-instances-cell {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--theme-fg);
}

.idle-instances-cwd {
  color: var(--theme-fg-subtle);
}

.idle-instances-agent {
  color: var(--theme-fg-dim);
}
</style>
