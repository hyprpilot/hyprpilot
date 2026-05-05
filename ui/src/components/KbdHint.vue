<script setup lang="ts">
import { isFaIcon, type KeyLabel } from '@components'

/**
 * Keyboard-hint chip. Keycaps are either plain text (Ctrl, Esc,
 * Ctrl+K) or directly-imported FontAwesome `IconDefinition`s rendered
 * as glyphs — the latter catches keys where system fonts give
 * inconsistent unicode (↑↓ ⏎ ⎋ ⇥).
 *
 * `size = 'sm'` (default) is the palette-footer chip size; `'md'` is
 * the focal-point variant for hero hints (idle-screen "Ctrl+K command
 * palette." prompt). Both consume the same theme tokens — the only
 * difference is type scale + keycap padding.
 */
withDefaults(
  defineProps<{
    keys: KeyLabel[]
    label: string
    size?: 'sm' | 'md'
  }>(),
  {
    size: 'sm'
  }
)
</script>

<template>
  <span class="kbd-hint" :data-size="size">
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
  @apply inline-flex items-center gap-1 leading-tight;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.kbd-hint[data-size='sm'] {
  @apply text-[0.7rem];
}

.kbd-hint[data-size='md'] {
  @apply gap-2 text-[0.85rem];
}

.kbd-hint-key {
  @apply inline-flex min-w-4 items-center justify-center border;
  color: var(--theme-fg-subtle);
  background-color: var(--theme-surface-alt);
  border-color: var(--theme-border);
}

.kbd-hint[data-size='sm'] .kbd-hint-key {
  @apply px-[4px] py-[1px] text-[0.68rem];
}

.kbd-hint[data-size='md'] .kbd-hint-key {
  @apply px-[6px] py-[2px] text-[0.82rem];
}

.kbd-hint-key-icon {
  width: 9px;
  height: 9px;
}

.kbd-hint[data-size='md'] .kbd-hint-key-icon {
  width: 11px;
  height: 11px;
}

.kbd-hint-label {
  color: var(--theme-fg-dim);
}
</style>
