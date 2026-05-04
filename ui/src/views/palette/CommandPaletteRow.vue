<script setup lang="ts">
import type { IconDefinition } from '@fortawesome/fontawesome-svg-core'

/**
 * Single palette row. Port of the wireframe's PaletteRow.
 *
 * design-skip: no fuzzy filter, no keyboard wiring. K-249 builds behaviour
 * on top of this shell.
 */
withDefaults(
  defineProps<{
    selected?: boolean
    icon?: IconDefinition
    label: string
    hint?: string
    right?: string
    danger?: boolean
  }>(),
  { selected: false, danger: false }
)

const emit = defineEmits<{
  select: []
  hover: []
}>()
</script>

<template>
  <button type="button" class="palette-row" :data-selected="selected" :data-danger="danger" @click="emit('select')" @mouseenter="emit('hover')">
    <span v-if="icon" class="palette-row-icon" aria-hidden="true">
      <FaIcon :icon="icon" />
    </span>
    <span class="palette-row-label">{{ label }}</span>
    <span v-if="hint" class="palette-row-hint">{{ hint }}</span>
    <span v-if="right" class="palette-row-right">{{ right }}</span>
  </button>
</template>

<style scoped>
@reference '../../assets/styles.css';

.palette-row {
  @apply flex w-full items-center gap-[10px] border-l-[3px] border-transparent bg-transparent px-[10px] py-[6px] text-[0.72rem] leading-tight;
  color: var(--theme-fg-subtle);
  cursor: pointer;
}

.palette-row[data-selected='true'] {
  background-color: var(--theme-surface-alt);
  border-left-color: var(--theme-status-warn);
}

.palette-row[data-danger='true'][data-selected='true'] {
  border-left-color: var(--theme-status-err);
}

.palette-row[data-danger='true'] .palette-row-icon {
  color: var(--theme-status-err);
}

.palette-row:hover {
  background-color: var(--theme-surface-alt);
}

.palette-row-icon {
  @apply inline-flex w-[18px] shrink-0 items-center justify-center text-[0.625rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.palette-row[data-selected='true'] .palette-row-icon {
  color: var(--theme-status-warn);
}

.palette-row-label {
  @apply shrink-0 text-[0.72rem];
  width: 120px;
  color: var(--theme-fg-subtle);
  font-family: var(--theme-font-mono);
}

.palette-row[data-selected='true'] .palette-row-label {
  color: var(--theme-fg);
  font-weight: 700;
}

.palette-row-hint {
  @apply flex-1 truncate text-[0.72rem];
  color: var(--theme-fg-subtle);
  font-family: var(--theme-font-sans);
}

.palette-row-right {
  @apply ml-2 shrink-0 text-[0.66rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}
</style>
