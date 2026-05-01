<script setup lang="ts">
/**
 * queue band — pinned above the composer when messages are queued.
 *
 * Header: "N queued · sent in order when pilot finishes" with a
 *         trailing line + global trash (drop-all).
 *
 * Rows: each is a green-stripe lane (captain message), index badge
 *       in user-soft bg + 3-icon action cluster (edit / send-now /
 *       drop). Same vocabulary as the rest of the surface — visual
 *       law #2 carries: green stripe = captain.
 */
import { faArrowUp, faPen, faTrash, faXmark } from '@fortawesome/free-solid-svg-icons'

import type { QueuedMessage } from '@components'

defineProps<{
  messages: QueuedMessage[]
}>()

const emit = defineEmits<{
  edit: [id: string]
  send: [id: string]
  drop: [id: string]
  dropAll: []
}>()
</script>

<template>
  <section v-if="messages.length > 0" class="queue-band" data-testid="queue-strip">
    <header class="queue-band-header">
      <span class="queue-band-count">{{ messages.length }}</span>
      <span class="queue-band-note">queued · sent in order when pilot finishes</span>
      <span class="queue-band-line" />
      <button
        type="button"
        class="queue-band-icon-btn"
        data-tone="err"
        title="drop all queued"
        aria-label="drop all queued"
        @click="emit('dropAll')"
      >
        <FaIcon :icon="faTrash" class="queue-band-icon" aria-hidden="true" />
      </button>
    </header>

    <ol class="queue-band-list">
      <li v-for="(m, idx) in messages" :key="m.id" class="queue-band-row">
        <span class="queue-band-index">{{ idx + 1 }}</span>
        <span class="queue-band-text">{{ m.text }}</span>
        <div class="queue-band-actions">
          <button
            type="button"
            class="queue-band-icon-btn"
            data-tone="warn"
            title="edit before sending"
            aria-label="edit"
            @click="emit('edit', m.id)"
          >
            <FaIcon :icon="faPen" class="queue-band-icon" aria-hidden="true" />
          </button>
          <button
            type="button"
            class="queue-band-icon-btn"
            data-tone="ok"
            title="send now (skip ahead)"
            aria-label="send now"
            @click="emit('send', m.id)"
          >
            <FaIcon :icon="faArrowUp" class="queue-band-icon" aria-hidden="true" />
          </button>
          <button
            type="button"
            class="queue-band-icon-btn"
            data-tone="err"
            title="drop from queue"
            aria-label="drop"
            @click="emit('drop', m.id)"
          >
            <FaIcon :icon="faXmark" class="queue-band-icon" aria-hidden="true" />
          </button>
        </div>
      </li>
    </ol>
  </section>
</template>

<style scoped>
@reference '../../assets/styles.css';

.queue-band {
  @apply flex flex-col;
  background-color: var(--theme-surface-bg);
  border-top: 1px solid var(--theme-border);
  padding: 8px 14px 6px 4px;
  gap: 5px;
}

.queue-band-header {
  @apply flex items-center text-[0.56rem] uppercase;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
  letter-spacing: 1px;
  padding-left: 4px;
  gap: 8px;
}

.queue-band-count {
  @apply font-bold;
  color: var(--theme-accent-user);
}

.queue-band-note {
  text-transform: lowercase;
  letter-spacing: normal;
}

.queue-band-line {
  @apply flex-1;
  height: 1px;
  background-color: var(--theme-border);
  margin-left: 4px;
}

.queue-band-list {
  @apply m-0 flex list-none flex-col gap-1 p-0;
}

.queue-band-row {
  @apply flex items-center gap-2;
  padding: 3px 8px;
  border-left: 2px solid var(--theme-accent-user);
  background-color: var(--theme-surface);
}

/* wireframe index badge: mono fontSize 10, fontWeight 700, user color on
 * user-soft bg, 16px min-width centered. */
.queue-band-index {
  @apply inline-flex shrink-0 items-center justify-center font-bold text-[0.6rem];
  background-color: var(--theme-accent-user-soft);
  color: var(--theme-accent-user);
  padding: 1px 6px;
  border-radius: 3px;
  min-width: 16px;
}

.queue-band-text {
  @apply min-w-0 flex-1 truncate text-[0.7rem];
  color: var(--theme-fg);
  line-height: 1.45;
}

.queue-band-actions {
  @apply flex shrink-0 items-center;
  gap: 2px;
}

/* wireframe iconBtn (22×22 ghost) — same shape as the permission panel
 * action buttons. */
.queue-band-icon-btn {
  @apply inline-flex items-center justify-center;
  width: 22px;
  height: 22px;
  padding: 0;
  border-radius: 3px;
  background-color: transparent;
  cursor: pointer;
}

.queue-band-icon-btn[data-tone='ok'] {
  border: 1px solid var(--theme-status-ok);
  color: var(--theme-status-ok);
}

.queue-band-icon-btn[data-tone='warn'] {
  border: 1px solid var(--theme-status-warn);
  color: var(--theme-status-warn);
}

.queue-band-icon-btn[data-tone='err'] {
  border: 1px solid var(--theme-status-err);
  color: var(--theme-status-err);
}

.queue-band-icon {
  width: 11px;
  height: 11px;
}

.queue-band-icon-btn:hover {
  filter: brightness(1.15);
}
</style>
