<script setup lang="ts">
import { currentId, modeRows, resultCount } from './palette-modes.fixture'
import { KbdHint, CommandPaletteRow, CommandPaletteShell } from '@components'
</script>

<template>
  <CommandPaletteShell>
    <template #title>
      <span class="palette-modes-breadcrumb">modes</span>
      <span class="palette-modes-sep">›</span>
      <span class="palette-modes-query"><span class="palette-modes-caret" aria-hidden="true" /></span>
      <span class="palette-modes-count">{{ resultCount }}</span>
    </template>
    <template #body>
      <CommandPaletteRow
        v-for="row in modeRows"
        :key="row.id"
        :icon="row.icon"
        :label="row.label"
        :hint="row.hint"
        :right="row.right"
        :danger="row.danger"
        :selected="row.id === currentId"
      />
    </template>
    <template #hints>
      <KbdHint :keys="[['fas', 'up-down']]" label="navigate" />
      <KbdHint :keys="[['fas', 'arrow-turn-down']]" label="confirm" />
      <KbdHint :keys="[['fas', 'circle-xmark']]" label="close" />
    </template>
  </CommandPaletteShell>
</template>

<style scoped>
@reference '../../../assets/styles.css';

.palette-modes-breadcrumb {
  color: var(--theme-fg-dim);
}

.palette-modes-sep {
  color: var(--theme-fg-dim);
}

.palette-modes-query {
  @apply flex-1;
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
  letter-spacing: normal;
}

.palette-modes-caret {
  @apply ml-[1px] inline-block h-[12px] w-[6px] animate-blink;
  background-color: var(--theme-fg);
  vertical-align: -1px;
}

.palette-modes-count {
  @apply ml-auto text-[0.56rem] normal-case;
  color: var(--theme-fg-dim);
  letter-spacing: normal;
}
</style>
