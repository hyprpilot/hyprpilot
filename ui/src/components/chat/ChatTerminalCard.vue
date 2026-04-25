<script setup lang="ts">
/**
 * Inline terminal card. Binds to `useTerminals().byId(terminalId)`
 * — Rust pushes stdout / stderr / exit chunks via `acp:terminal`,
 * the composable accumulates them, and this card renders the
 * scrollback. Status dot reads as streaming → `state.stream`,
 * clean exit → `status.ok`, non-zero or signal → `status.err`.
 *
 * Output is filtered through a tiny ANSI subset (color escapes +
 * `\x1b[2K` clear-line) — pilot's behavior is the floor; we
 * deliberately don't pull a full ANSI library.
 */
import { computed } from 'vue'

import { stripAnsi } from './ansi'
import { useTerminals } from '@composables/use-terminals'


const props = defineProps<{
  terminalId: string
  /** Override the active instance — passes through to `useTerminals(instanceId)`. */
  instanceId?: string
}>()

const emit = defineEmits<{
  cancel: []
}>()

const entry = useTerminals(props.instanceId).byId(props.terminalId)

const command = computed(() => entry.value?.command ?? '')
const cwd = computed(() => entry.value?.cwd)
const output = computed(() => stripAnsi(entry.value?.output ?? ''))
const running = computed(() => entry.value?.running ?? false)
const truncated = computed(() => entry.value?.truncated ?? false)
const exitCode = computed(() => entry.value?.exitCode)
const signal = computed(() => entry.value?.signal)

const exitOk = computed(() => exitCode.value === 0 && !signal.value)
const exitLabel = computed(() => {
  if (signal.value) {
    return `signal ${signal.value}`
  }
  if (exitCode.value !== undefined) {
    return `exit ${exitCode.value}`
  }

  return ''
})
</script>

<template>
  <section class="terminal-card" data-testid="terminal-card" :data-running="running">
    <header class="terminal-card-header">
      <FaIcon :icon="['fas', 'terminal']" class="terminal-card-kind" aria-hidden="true" />
      <span class="terminal-card-label">Bash</span>
      <code class="terminal-card-command">{{ command || terminalId }}</code>
      <span v-if="cwd" class="terminal-card-cwd">· {{ cwd }}</span>
      <span class="terminal-card-status-dot" :data-state="running ? 'stream' : exitOk ? 'ok' : 'err'" aria-hidden="true" />
      <button v-if="running" type="button" class="terminal-card-cancel" @click="emit('cancel')">cancel</button>
      <span v-else-if="exitLabel" class="terminal-card-exit" :data-ok="exitOk">{{ exitLabel }}</span>
    </header>

    <pre
      class="terminal-card-stdout"
    ><span v-if="truncated" class="terminal-card-truncated">… (older output dropped)
</span><span class="terminal-card-stdout-text">{{ output }}</span><span v-if="running" class="terminal-card-cursor" aria-hidden="true">▊</span></pre>
  </section>
</template>

<style scoped>
@reference '../../assets/styles.css';

.terminal-card {
  @apply flex min-w-0 flex-col overflow-hidden border;
  border-color: var(--theme-border);
  background-color: var(--theme-surface-bg);
}

.terminal-card-header {
  @apply flex min-w-0 items-center gap-2 border-b px-2 py-[5px] text-[0.72rem];
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface-alt);
  font-family: var(--theme-font-mono);
}

.terminal-card-label {
  @apply font-bold;
  color: var(--theme-kind-bash);
}

.terminal-card-kind {
  @apply shrink-0;
  width: 11px;
  height: 11px;
  color: var(--theme-kind-bash);
}

.terminal-card-command {
  @apply min-w-0 flex-1 truncate;
  color: var(--theme-fg-ink-2);
  font-family: var(--theme-font-mono);
}

.terminal-card-cwd {
  color: var(--theme-fg-dim);
}

.terminal-card-status-dot {
  @apply inline-block h-[6px] w-[6px] shrink-0 rounded-full;
}

.terminal-card-status-dot[data-state='stream'] {
  background-color: var(--theme-state-stream);
  @apply animate-pulse-slow;
}

.terminal-card-status-dot[data-state='ok'] {
  background-color: var(--theme-status-ok);
}

.terminal-card-status-dot[data-state='err'] {
  background-color: var(--theme-status-err);
}

.terminal-card-cancel {
  @apply border-0 bg-transparent px-1 text-[0.7rem];
  color: var(--theme-status-err);
  cursor: pointer;
}

.terminal-card-cancel:hover {
  text-decoration: underline;
}

.terminal-card-exit {
  @apply text-[0.7rem];
  color: var(--theme-status-err);
}

.terminal-card-exit[data-ok='true'] {
  color: var(--theme-status-ok);
}

.terminal-card-stdout {
  @apply m-0 overflow-auto px-2 py-2 text-[0.76rem] leading-snug;
  color: var(--theme-fg-ink-2);
  background-color: var(--theme-surface-bg);
  font-family: var(--theme-font-mono);
  white-space: pre-wrap;
  max-height: 16rem;
}

.terminal-card-truncated {
  color: var(--theme-fg-dim);
  font-style: italic;
}

.terminal-card-cursor {
  @apply inline-block animate-blink;
  color: var(--theme-state-stream);
}
</style>
