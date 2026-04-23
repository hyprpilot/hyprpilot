<script setup lang="ts">
/**
 * Queued-message strip. Colored-left-border rows with index + text + row
 * actions. Header: `QUEUED · N` · FIFO on turn-complete · in-memory
 * only · drop all. Port of the inline queue widget in `D5_Queue`.
 */
import type { QueuedMessage } from '../types'

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
  <section v-if="messages.length > 0" class="queue-strip" data-testid="queue-strip">
    <header class="queue-strip-header">
      <span class="queue-strip-title">QUEUED · {{ messages.length }}</span>
      <span class="queue-strip-sep">·</span>
      <span class="queue-strip-note">FIFO on turn-complete</span>
      <span class="queue-strip-sep">·</span>
      <span class="queue-strip-note">in-memory only</span>
      <button type="button" class="queue-strip-drop-all" @click="emit('dropAll')">drop all</button>
    </header>

    <ol class="queue-strip-list">
      <li v-for="(m, idx) in messages" :key="m.id" class="queue-strip-row">
        <span class="queue-strip-index">{{ idx + 1 }}</span>
        <span class="queue-strip-text">{{ m.text }}</span>
        <span class="queue-strip-actions">
          <button type="button" class="queue-strip-action is-edit" @click="emit('edit', m.id)">edit</button>
          <button type="button" class="queue-strip-action is-send" @click="emit('send', m.id)">send</button>
          <button type="button" class="queue-strip-action is-drop" @click="emit('drop', m.id)">drop</button>
        </span>
      </li>
    </ol>
  </section>
</template>

<style scoped>
@reference '../../assets/styles.css';

.queue-strip {
  @apply flex flex-col gap-[3px] py-[6px] px-[14px];
  background-color: var(--theme-surface-bg);
  border-top: 1px solid var(--theme-border);
}

.queue-strip-header {
  @apply flex items-baseline gap-2 pb-1 text-[0.56rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
  letter-spacing: 1px;
}

.queue-strip-title {
  @apply font-bold;
  color: var(--theme-fg-dim);
}

.queue-strip-sep {
  @apply text-[0.62rem];
  color: var(--theme-fg-dim);
  letter-spacing: 0;
}

.queue-strip-note {
  @apply text-[0.62rem] normal-case;
  color: var(--theme-fg-dim);
  letter-spacing: 0;
}

.queue-strip-drop-all {
  @apply ml-auto border-0 bg-transparent px-1 text-[0.62rem];
  color: var(--theme-status-err);
  font-family: var(--theme-font-mono);
  cursor: pointer;
  letter-spacing: 0;
}

.queue-strip-drop-all:hover {
  text-decoration: underline;
}

.queue-strip-list {
  @apply m-0 flex list-none flex-col gap-[3px] p-0;
}

.queue-strip-row {
  @apply flex items-center gap-[10px] border-l-[3px] px-[10px] py-[5px] text-[0.72rem];
  background-color: var(--theme-surface);
  border-color: var(--theme-accent-user);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  font-family: var(--theme-font-sans);
}

.queue-strip-index {
  @apply shrink-0 text-[0.62rem] font-bold;
  width: 14px;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.queue-strip-text {
  @apply flex-1 truncate;
  color: var(--theme-fg);
}

.queue-strip-actions {
  @apply ml-auto inline-flex shrink-0 items-center gap-2;
}

.queue-strip-action {
  @apply border-0 bg-transparent px-1 text-[0.62rem];
  font-family: var(--theme-font-mono);
  cursor: pointer;
}

.queue-strip-action.is-edit {
  color: var(--theme-status-warn);
}

.queue-strip-action.is-send {
  color: var(--theme-status-ok);
}

.queue-strip-action.is-drop {
  color: var(--theme-status-err);
}

.queue-strip-action:hover {
  text-decoration: underline;
}
</style>
