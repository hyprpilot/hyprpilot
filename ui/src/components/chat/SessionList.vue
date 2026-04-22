<script setup lang="ts">
import type { SessionSummary } from '@ipc'

defineProps<{
  sessions: SessionSummary[]
  loading?: boolean
  activeSessionId?: string
}>()

const emit = defineEmits<{
  load: [sessionId: string]
}>()
</script>

<template>
  <aside class="session-list" data-testid="session-list">
    <header class="session-list-header">
      <span class="session-list-title">sessions</span>
    </header>

    <p v-if="loading" class="session-list-empty">loading…</p>
    <p v-else-if="sessions.length === 0" class="session-list-empty" data-testid="session-list-empty">no saved sessions yet</p>

    <ul v-else class="session-list-items">
      <li v-for="s in sessions" :key="s.sessionId" class="session-list-item" :class="{ 'session-list-item-active': s.sessionId === activeSessionId }">
        <button
          type="button"
          class="session-list-button"
          :data-testid="`session-list-item-${s.sessionId}`"
          :aria-current="s.sessionId === activeSessionId ? 'true' : undefined"
          @click="s.sessionId !== activeSessionId && emit('load', s.sessionId)"
        >
          <span class="session-list-item-title">{{ s.title ?? s.sessionId }}</span>
          <span v-if="s.updatedAt" class="session-list-item-meta">{{ s.updatedAt }}</span>
        </button>
      </li>
    </ul>
  </aside>
</template>

<style scoped>
@reference "../../assets/styles.css";

.session-list {
  @apply flex w-40 flex-col gap-1 border-r px-2 py-2;
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface-card-user);
  color: var(--theme-fg);
  font-family: var(--theme-font-family);
}

.session-list-header {
  @apply flex items-center justify-between text-[0.7rem] uppercase tracking-wider;
  color: var(--theme-fg-muted);
}

.session-list-title {
  @apply font-bold;
}

.session-list-empty {
  @apply m-0 text-[0.75rem];
  color: var(--theme-fg-muted);
}

.session-list-items {
  @apply m-0 flex list-none flex-col gap-1 p-0;
}

.session-list-item {
  @apply flex;
}

.session-list-button {
  @apply flex w-full cursor-pointer flex-col items-start gap-0.5 border px-2 py-1 text-left text-[0.8rem];
  background-color: transparent;
  color: var(--theme-fg);
  border-color: transparent;

  &:hover {
    border-color: var(--theme-border-soft);
    background-color: var(--theme-surface-compose);
  }

  &:focus-visible {
    outline: none;
    border-color: var(--theme-border-focus);
  }
}

.session-list-item-active .session-list-button {
  border-color: var(--theme-accent);
  background-color: var(--theme-surface-compose);
}

.session-list-item-title {
  @apply truncate;
  color: var(--theme-fg);
  max-width: 100%;
}

.session-list-item-meta {
  @apply text-[0.7rem] uppercase tracking-wider;
  color: var(--theme-fg-muted);
}
</style>
