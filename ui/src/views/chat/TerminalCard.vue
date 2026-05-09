<script setup lang="ts">
/**
 * Inline terminal card. Binds to `useTerminals().byId(terminalId)`
 * — Rust pushes stdout / stderr / exit chunks via `acp:terminal`,
 * the composable accumulates them, and this card renders the
 * scrollback through an xterm.js viewer (ANSI escape sequences
 * render as colors / cursor moves / clear-line — the way a real
 * terminal would). Previously we ran the output through a tiny
 * `stripAnsi` shim and dumped it into a `<pre>`, which lost
 * everything except the literal characters. Status dot reads as
 * streaming → `state.stream`, clean exit → `status.ok`, non-zero
 * or signal → `status.err`.
 */
import { faTerminal } from '@fortawesome/free-solid-svg-icons'
import { computed } from 'vue'

import { XtermView } from '@components'
import { useTerminals } from '@composables'

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
const output = computed(() => entry.value?.output ?? '')
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
      <FaIcon :icon="faTerminal" class="terminal-card-kind" aria-hidden="true" />
      <span class="terminal-card-label">Bash</span>
      <code class="terminal-card-command">{{ command || terminalId }}</code>
      <span v-if="cwd" class="terminal-card-cwd">· {{ cwd }}</span>
      <span class="terminal-card-status-dot" :data-state="running ? 'stream' : exitOk ? 'ok' : 'err'" aria-hidden="true" />
      <button v-if="running" type="button" class="terminal-card-cancel" @click="emit('cancel')">cancel</button>
      <span v-else-if="exitLabel" class="terminal-card-exit" :data-ok="exitOk">{{ exitLabel }}</span>
    </header>

    <div class="terminal-card-body">
      <p v-if="truncated" class="terminal-card-truncated">… (older output dropped)</p>
      <XtermView class="terminal-card-xterm" :text="output" :running="running" :rows="16" />
    </div>
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
  @apply flex min-w-0 items-center gap-2 border-b px-2 py-[0.3125rem] text-[0.72rem];
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface-alt);
  font-family: var(--theme-font-mono);
}

.terminal-card-label {
  @apply shrink-0 font-bold;
  color: var(--theme-kind-bash);
}

.terminal-card-kind {
  @apply shrink-0;
  width: 0.6875rem;
  height: 0.6875rem;
  color: var(--theme-kind-bash);
}

.terminal-card-command {
  @apply min-w-0 flex-1 truncate;
  color: var(--theme-fg-subtle);
  font-family: var(--theme-font-mono);
}

/* `cwd` shrinks before `command` (deeper paths are less informative
 * than the command itself); both yield before the trailing status
 * dot + cancel button, which carry hard `shrink-0`. */
.terminal-card-cwd {
  @apply min-w-0 shrink truncate;
  max-width: 40%;
  color: var(--theme-fg-dim);
}

.terminal-card-cancel,
.terminal-card-exit {
  @apply shrink-0;
}

.terminal-card-status-dot {
  @apply inline-block h-[0.375rem] w-[0.375rem] shrink-0 rounded-full;
}

.terminal-card-status-dot[data-state='stream'] {
  background-color: var(--theme-state-stream);
  @apply animate-pulse;
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

@media (pointer: coarse) {
  .terminal-card-cancel {
    min-height: 2.25rem;
    padding: 0 0.625rem;
  }
}

.terminal-card-exit {
  @apply text-[0.7rem];
  color: var(--theme-status-err);
}

.terminal-card-exit[data-ok='true'] {
  color: var(--theme-status-ok);
}

.terminal-card-body {
  @apply flex min-w-0 flex-col;
  background-color: var(--theme-surface-bg);
}

.terminal-card-truncated {
  @apply m-0 px-2 py-1 text-[0.7rem];
  color: var(--theme-fg-dim);
  font-style: italic;
  border-bottom: 1px dashed var(--theme-border-soft);
}

.terminal-card-xterm {
  /* xterm host owns its own padding + theming via the component;
   * card-level borders come from the section wrapper. */
}
</style>
