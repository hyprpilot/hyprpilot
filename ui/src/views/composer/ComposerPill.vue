<script setup lang="ts">
/**
 * Single pill chip inside the composer row. Presentation-only — the
 * parent `ChatComposer` owns state + emits removal intent.
 *
 * `kind` drives the `data-kind` attribute so future stylistic
 * distinctions (resource vs. attachment colouring) stay a CSS concern
 * without re-threading props.
 */

import { faXmark } from '@fortawesome/free-solid-svg-icons'

import type { ComposerPill } from '@components'

defineProps<{ pill: ComposerPill }>()
defineEmits<{ remove: [id: string] }>()
</script>

<template>
  <span class="composer-pill" :data-kind="pill.kind">
    <span class="composer-pill-label">{{ pill.label }}</span>
    <button type="button" class="composer-pill-remove" aria-label="remove" @click="$emit('remove', pill.id)">
      <FaIcon :icon="faXmark" class="composer-pill-remove-icon" />
    </button>
  </span>
</template>

<style scoped>
@reference '../../assets/styles.css';

.composer-pill {
  @apply inline-flex items-center gap-1 border px-2 py-[2px] text-[0.7rem] leading-tight;
  font-family: var(--theme-font-mono);
  color: var(--theme-fg-ink-2);
  background-color: var(--theme-surface-alt);
  border-color: var(--theme-border);
}

.composer-pill[data-kind='attachment'] {
  border-color: var(--theme-accent-assistant-soft);
}

.composer-pill-label {
  @apply truncate;
  max-width: 24ch;
}

.composer-pill-remove {
  @apply border-0 bg-transparent px-0 text-[0.7rem] leading-none;
  color: var(--theme-fg-dim);
  cursor: pointer;
}

.composer-pill-remove-icon {
  width: 9px;
  height: 9px;
}

.composer-pill-remove:hover {
  color: var(--theme-status-err);
}
</style>
