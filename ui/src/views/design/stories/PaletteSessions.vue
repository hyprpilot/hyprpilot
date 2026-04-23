<script setup lang="ts">
import { computed } from 'vue'

import { phaseForState, resultCount, selectedId, sessionRows } from './palette-sessions.fixture'
import { Button, KbdHint, CommandPaletteRow, CommandPaletteShell, ButtonTone, ButtonVariant } from '@components'

const selected = computed(() => sessionRows.find((s) => s.id === selectedId))
</script>

<template>
  <CommandPaletteShell>
    <template #title>
      <span class="palette-sessions-breadcrumb">sessions</span>
      <span class="palette-sessions-sep">›</span>
      <span class="palette-sessions-query"><span class="palette-sessions-caret" aria-hidden="true" /></span>
      <span class="palette-sessions-count">{{ resultCount }}</span>
    </template>
    <template #body>
      <CommandPaletteRow
        v-for="row in sessionRows"
        :key="row.id"
        :data-phase="phaseForState(row.state)"
        :icon="['fas', 'circle']"
        :label="row.profile"
        :hint="row.title"
        :right="row.t"
        :selected="row.id === selectedId"
      />
    </template>
    <template #preview>
      <div v-if="selected" class="palette-sessions-preview">
        <div class="palette-sessions-preview-state">
          <span class="palette-sessions-preview-dot" :data-state="selected.state" aria-hidden="true" />
          <span class="palette-sessions-preview-state-label">{{ selected.state }}</span>
          <span class="palette-sessions-preview-state-sep">· {{ selected.t }}</span>
        </div>

        <h3 class="palette-sessions-preview-title">{{ selected.title }}</h3>
        <div class="palette-sessions-preview-adapter">{{ selected.profile }} <span class="palette-sessions-preview-sep-inline">·</span> {{ selected.meta }}</div>

        <dl class="palette-sessions-preview-meta">
          <div>
            <dt>cwd</dt>
            <dd>~/dev/hyprpilot</dd>
          </div>
          <div>
            <dt>turns</dt>
            <dd>{{ selected.turns }}</dd>
          </div>
          <div>
            <dt>state</dt>
            <dd :data-state="selected.state">{{ selected.live ? 'live · in-memory' : 'paused · resumable from log' }}</dd>
          </div>
        </dl>

        <div class="palette-sessions-preview-recent-label">RECENT</div>
        <div class="palette-sessions-preview-recent">
          <div class="palette-sessions-preview-turn" data-role="user">
            <div class="palette-sessions-preview-turn-tag">captain</div>
            <div class="palette-sessions-preview-turn-body">reanalyze daemon/mod.rs and come up with the refactor plan</div>
          </div>
          <div class="palette-sessions-preview-turn" data-role="assistant">
            <div class="palette-sessions-preview-turn-tag">pilot</div>
            <div class="palette-sessions-preview-turn-body">extracting tools/fs.rs — 5 steps queued, currently writing tools/terminal.rs</div>
          </div>
        </div>

        <div class="palette-sessions-preview-actions">
          <Button :variant="ButtonVariant.Solid" :tone="ButtonTone.Ok">{{ selected.live ? 'focus' : 'resume' }}</Button>
          <Button :variant="ButtonVariant.Ghost" :tone="ButtonTone.Neutral">fork</Button>
          <Button :variant="ButtonVariant.Ghost" :tone="ButtonTone.Err">kill</Button>
        </div>
      </div>
    </template>
    <template #hints>
      <KbdHint :keys="[['fas', 'up-down']]" label="navigate" />
      <KbdHint :keys="[['fas', 'arrow-turn-down']]" label="focus" />
      <KbdHint :keys="['⌘', ['fas', 'arrow-turn-down']]" label="resume" />
      <KbdHint :keys="['Ctrl', 'N']" label="new" />
      <KbdHint :keys="[['fas', 'circle-xmark']]" label="close" />
    </template>
  </CommandPaletteShell>
</template>

<style scoped>
@reference '../../../assets/styles.css';

.palette-sessions-breadcrumb {
  color: var(--theme-fg-dim);
}

.palette-sessions-sep {
  color: var(--theme-fg-dim);
}

.palette-sessions-query {
  @apply flex-1;
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
  letter-spacing: normal;
}

.palette-sessions-caret {
  @apply ml-[1px] inline-block h-[12px] w-[6px] animate-blink;
  background-color: var(--theme-fg);
  vertical-align: -1px;
}

.palette-sessions-count {
  @apply ml-auto text-[0.56rem] normal-case;
  color: var(--theme-fg-dim);
  letter-spacing: normal;
}

.palette-sessions-preview {
  @apply flex flex-col gap-1 px-[14px] py-[12px];
}

.palette-sessions-preview-state {
  @apply mb-[6px] flex items-center gap-[6px];
}

.palette-sessions-preview-dot {
  @apply inline-block h-[8px] w-[8px] rounded-full;
  background-color: var(--theme-fg-dim);
}

.palette-sessions-preview-dot[data-state='streaming'] {
  @apply animate-pulse-slow;
  background-color: var(--theme-state-stream);
}

.palette-sessions-preview-dot[data-state='awaiting'] {
  background-color: var(--theme-state-awaiting);
}

.palette-sessions-preview-dot[data-state='idle'] {
  background-color: var(--theme-status-ok);
}

.palette-sessions-preview-state-label {
  @apply text-[0.56rem] uppercase;
  color: var(--theme-fg-ink-2);
  font-family: var(--theme-font-mono);
  letter-spacing: 1px;
}

.palette-sessions-preview-state-sep {
  @apply text-[0.62rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.palette-sessions-preview-title {
  @apply m-0 text-left text-[1rem] font-semibold leading-tight;
  color: var(--theme-fg);
  letter-spacing: -0.1px;
}

.palette-sessions-preview-adapter {
  @apply text-left text-[0.72rem] font-semibold;
  color: var(--theme-accent);
  font-family: var(--theme-font-mono);
}

.palette-sessions-preview-sep-inline {
  color: var(--theme-fg-dim);
}

.palette-sessions-preview-meta {
  @apply m-0 mt-[6px] grid gap-y-[3px] gap-x-[10px] text-[0.62rem];
  font-family: var(--theme-font-mono);
  grid-template-columns: auto 1fr;
}

.palette-sessions-preview-meta > div {
  @apply contents;
}

.palette-sessions-preview-meta dt {
  color: var(--theme-fg-dim);
}

.palette-sessions-preview-meta dd {
  @apply m-0;
  color: var(--theme-fg-ink-2);
}

.palette-sessions-preview-meta dd[data-state='streaming'] {
  color: var(--theme-state-stream);
}

.palette-sessions-preview-meta dd[data-state='awaiting'] {
  color: var(--theme-state-awaiting);
}

.palette-sessions-preview-meta dd[data-state='idle'] {
  color: var(--theme-status-ok);
}

.palette-sessions-preview-meta dd[data-state='paused'] {
  color: var(--theme-fg-dim);
}

.palette-sessions-preview-recent-label {
  @apply mt-[12px] text-[0.56rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
  letter-spacing: 1px;
}

.palette-sessions-preview-recent {
  @apply mt-[6px] flex flex-col gap-[6px];
}

.palette-sessions-preview-turn {
  @apply pl-2;
  border-left: 2px solid var(--theme-accent-user);
}

.palette-sessions-preview-turn[data-role='assistant'] {
  border-left-color: var(--theme-accent-assistant);
}

.palette-sessions-preview-turn-tag {
  @apply mb-[3px] inline-block rounded-sm px-[6px] py-[1px] text-[0.56rem] font-bold;
  font-family: var(--theme-font-mono);
  background-color: var(--theme-accent-user-soft);
  color: var(--theme-accent-user);
  letter-spacing: 0.4px;
}

.palette-sessions-preview-turn[data-role='assistant'] .palette-sessions-preview-turn-tag {
  background-color: var(--theme-accent-assistant-soft);
  color: var(--theme-accent-assistant);
}

.palette-sessions-preview-turn-body {
  @apply text-[0.72rem] leading-snug;
  color: var(--theme-fg);
}

.palette-sessions-preview-actions {
  @apply mt-[12px] flex gap-[6px];
}
</style>
