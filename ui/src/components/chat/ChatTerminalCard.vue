<script setup lang="ts">
/**
 * Running-bash / terminal card: streaming <pre> stdout with a blinking
 * cursor + cancel link. Serves both the inline terminal variant of
 * `D5BigToolRow` and the standalone `ToolCallContent::Terminal` surface.
 * The cancel link is presentational — parent decides what `cancel` means.
 */
withDefaults(
  defineProps<{
    command: string
    cwd?: string
    stdout: string
    cancellable?: boolean
    running?: boolean
    exitCode?: number
  }>(),
  { cancellable: true, running: true }
)

const emit = defineEmits<{
  cancel: []
}>()
</script>

<template>
  <section class="terminal-card" data-testid="terminal-card" :data-running="running">
    <header class="terminal-card-header">
      <FaIcon :icon="['fas', 'terminal']" class="terminal-card-kind" aria-hidden="true" />
      <span class="terminal-card-label">Bash</span>
      <code class="terminal-card-command">{{ command }}</code>
      <span v-if="cwd" class="terminal-card-cwd">· {{ cwd }}</span>
      <button v-if="cancellable && running" type="button" class="terminal-card-cancel" @click="emit('cancel')">cancel</button>
      <span v-else-if="!running && exitCode !== undefined" class="terminal-card-exit" :data-ok="exitCode === 0">exit {{ exitCode }}</span>
    </header>

    <pre
      class="terminal-card-stdout"
    ><span class="terminal-card-stdout-text">{{ stdout }}</span><span v-if="running" class="terminal-card-cursor" aria-hidden="true">▊</span></pre>
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

.terminal-card-cursor {
  @apply inline-block animate-blink;
  color: var(--theme-state-stream);
}
</style>
