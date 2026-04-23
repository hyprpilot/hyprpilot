<script setup lang="ts">
import { isFaIcon, type KeyLabel } from './types'

/**
 * Keyboard-hint chip. Keycaps are either plain text (Ctrl, Esc, Ctrl+K)
 * or `FaIconSpec` tuples rendered as FontAwesome glyphs — the latter
 * catches keys where system fonts give inconsistent unicode (↑↓ ⏎ ⎋ ⇥).
 * Port of D5's `KbdHint`. Stateless.
 */
defineProps<{
  keys: KeyLabel[]
  label: string
}>()
</script>

<template>
  <span class="kbd-hint">
    <kbd v-for="(k, i) in keys" :key="i" class="kbd-hint-key">
      <FaIcon v-if="isFaIcon(k)" :icon="k" class="kbd-hint-key-icon" />
      <template v-else>{{ k }}</template>
    </kbd>
    <span class="kbd-hint-label">{{ label }}</span>
  </span>
</template>

<style scoped>
@reference '../assets/styles.css';

.kbd-hint {
  @apply inline-flex items-center gap-1 text-[0.7rem] leading-tight;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.kbd-hint-key {
  @apply inline-flex min-w-4 items-center justify-center border px-[4px] py-[1px] text-[0.68rem];
  color: var(--theme-fg-ink-2);
  background-color: var(--theme-surface-alt);
  border-color: var(--theme-border);
}

.kbd-hint-key-icon {
  width: 9px;
  height: 9px;
}

.kbd-hint-label {
  color: var(--theme-fg-dim);
}
</style>
