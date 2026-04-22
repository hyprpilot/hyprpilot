<script setup lang="ts">
import { computed } from 'vue'

import { type ProfileSummary, SessionState, type SessionStateEvent } from '@composables'

const props = defineProps<{
  profiles: ProfileSummary[]
  selectedProfileId?: string
  state?: SessionStateEvent
}>()

const emit = defineEmits<{
  selectProfile: [id: string]
}>()

const activeProfile = computed(() => props.profiles.find((p) => p.id === props.selectedProfileId))

const stateClass = computed(() => {
  const s = props.state?.state
  switch (s) {
    case SessionState.Running:
    case SessionState.Starting:
      return 'status-dot-running'
    case SessionState.Error:
      return 'status-dot-error'
    case SessionState.Ended:
    default:
      return 'status-dot-idle'
  }
})

function onProfileChange(e: Event): void {
  const value = (e.target as HTMLSelectElement).value
  if (value) emit('selectProfile', value)
}
</script>

<template>
  <div class="status-strip" data-testid="status-strip">
    <span class="status-dot" :class="stateClass" aria-hidden="true" />
    <span class="status-label">{{ state?.state ?? 'idle' }}</span>

    <span v-if="activeProfile" class="status-model" data-testid="status-model">
      {{ activeProfile.agent }}<span v-if="activeProfile.model">/{{ activeProfile.model }}</span>
    </span>

    <label class="status-profile">
      <span class="status-profile-label">profile</span>
      <select class="status-profile-select" data-testid="status-profile-select" :value="selectedProfileId ?? ''" :disabled="profiles.length === 0" @change="onProfileChange">
        <option v-if="profiles.length === 0" disabled value="">no profiles</option>
        <option v-for="p in profiles" :key="p.id" :value="p.id">{{ p.id }}{{ p.is_default ? ' (default)' : '' }}</option>
      </select>
    </label>
  </div>
</template>

<style scoped>
@reference "../../assets/styles.css";

.status-strip {
  @apply flex items-center gap-3 border-b px-3 py-1 text-[0.75rem] uppercase tracking-wider;
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface-compose);
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-family);
}

.status-dot {
  @apply h-2 w-2 rounded-full;
}

.status-dot-idle {
  background-color: var(--theme-state-idle);
}

.status-dot-running {
  background-color: var(--theme-state-stream);
}

.status-dot-error {
  background-color: var(--theme-state-pending);
}

.status-label {
  color: var(--theme-fg);
}

.status-model {
  @apply normal-case;
  color: var(--theme-fg-muted);
}

.status-profile {
  @apply ml-auto flex items-center gap-2;
}

.status-profile-label {
  color: var(--theme-fg-muted);
}

.status-profile-select {
  @apply border px-2 py-0.5 text-[0.75rem] lowercase;
  background-color: var(--theme-window);
  color: var(--theme-fg);
  border-color: var(--theme-border-soft);
  font-family: var(--theme-font-family);

  &:focus {
    outline: none;
    border-color: var(--theme-border-focus);
  }

  &:disabled {
    opacity: 0.5;
  }
}
</style>
