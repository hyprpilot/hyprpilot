<script setup lang="ts">
import { paletteQuery, resultCount, rootRows, selectedId } from './palette-root.fixture'
import { KbdHint, CommandPaletteRow, CommandPaletteShell } from '@components'
</script>

<template>
  <CommandPaletteShell>
    <template #title>
      <span class="palette-root-breadcrumb">palette</span>
      <span class="palette-root-sep">›</span>
      <span class="palette-root-query">{{ paletteQuery }}<span class="palette-root-caret" aria-hidden="true" /></span>
      <span class="palette-root-count">{{ resultCount }}</span>
    </template>
    <template #body>
      <CommandPaletteRow
        v-for="row in rootRows"
        :key="row.id"
        :icon="row.icon"
        :label="row.label"
        :hint="row.hint"
        :right="row.right"
        :danger="row.danger"
        :selected="row.id === selectedId"
      />
    </template>
    <template #hints>
      <KbdHint :keys="[['fas', 'up-down']]" label="navigate" />
      <KbdHint :keys="[['fas', 'arrow-right-to-bracket']]" label="multi" />
      <KbdHint :keys="[['fas', 'arrow-turn-down']]" label="confirm" />
      <KbdHint :keys="['Ctrl', 'D']" label="delete" />
      <KbdHint :keys="[['fas', 'circle-xmark']]" label="close" />
    </template>
  </CommandPaletteShell>
</template>

<style scoped>
@reference '../../../assets/styles.css';

.palette-root-breadcrumb {
  color: var(--theme-fg-dim);
}

.palette-root-sep {
  color: var(--theme-fg-dim);
}

.palette-root-query {
  @apply flex-1 normal-case;
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
  letter-spacing: normal;
}

.palette-root-caret {
  @apply ml-[1px] inline-block h-[12px] w-[6px] animate-blink;
  background-color: var(--theme-fg);
  vertical-align: -1px;
}

.palette-root-count {
  @apply ml-auto text-[0.56rem] normal-case;
  color: var(--theme-fg-dim);
  letter-spacing: normal;
}
</style>
