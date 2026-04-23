<script setup lang="ts">
import { resultCount, selectedIds, skillRows } from './palette-pickers.fixture'
import { KbdHint, CommandPaletteRow, CommandPaletteShell } from '@components'
</script>

<template>
  <CommandPaletteShell>
    <template #title>
      <span class="palette-pickers-breadcrumb">skills</span>
      <span class="palette-pickers-sep">›</span>
      <span class="palette-pickers-query"><span class="palette-pickers-caret" aria-hidden="true" /></span>
      <span class="palette-pickers-count">{{ resultCount }}</span>
    </template>
    <template #body>
      <CommandPaletteRow v-for="row in skillRows" :key="row.id" :icon="row.icon" :label="row.label" :hint="row.hint" :right="row.right" :selected="selectedIds.has(row.id)" />
    </template>
    <template #hints>
      <KbdHint :keys="[['fas', 'up-down']]" label="navigate" />
      <KbdHint :keys="[['fas', 'arrow-right-to-bracket']]" label="toggle" />
      <KbdHint :keys="[['fas', 'arrow-turn-down']]" label="confirm" />
      <KbdHint :keys="[['fas', 'circle-xmark']]" label="close" />
    </template>
  </CommandPaletteShell>
</template>

<style scoped>
@reference '../../../assets/styles.css';

.palette-pickers-breadcrumb {
  color: var(--theme-fg-dim);
}

.palette-pickers-sep {
  color: var(--theme-fg-dim);
}

.palette-pickers-query {
  @apply flex-1;
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
  letter-spacing: normal;
}

.palette-pickers-caret {
  @apply ml-[1px] inline-block h-[12px] w-[6px] animate-blink;
  background-color: var(--theme-fg);
  vertical-align: -1px;
}

.palette-pickers-count {
  @apply ml-auto text-[0.56rem] normal-case;
  color: var(--theme-fg-dim);
  letter-spacing: normal;
}
</style>
