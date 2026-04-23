<script setup lang="ts">
import { computed } from 'vue'

import BreadcrumbPill from './BreadcrumbPill.vue'
import Pill from './Pill.vue'
import { Phase, type BreadcrumbCount, type GitStatus } from './types'

/**
 * Overlay chrome: header row 1 (profile pill + mode + provider/model +
 * title + close), header row 2 (cwd expand + breadcrumb counts), optional
 * toast slot, default slot for body, composer slot. Port of D5's `D5Frame`.
 *
 * Pure presentational — `close` emit is the only behaviour; parent decides
 * what closing means.
 */
const props = withDefaults(
  defineProps<{
    profile: string
    phase?: Phase
    modeTag?: string
    provider?: string
    model?: string
    title?: string
    cwd?: string
    gitStatus?: GitStatus
    counts?: BreadcrumbCount[]
    cwdExpanded?: boolean
  }>(),
  {
    phase: Phase.Idle,
    modeTag: 'ask',
    counts: () => [],
    cwdExpanded: false
  }
)

const emit = defineEmits<{
  close: []
  toggleCwd: []
}>()

// Rust exposes the streaming state as `stream` (see `config.state.stream`);
// every other phase value already matches its CSS suffix 1:1.
function phaseToCssSuffix(p: Phase): string {
  return p === Phase.Streaming ? 'stream' : p.toLowerCase()
}

const phaseColor = computed(() => `var(--theme-state-${phaseToCssSuffix(props.phase)})`)
const isPulsing = computed(() => props.phase === Phase.Streaming || props.phase === Phase.Working)
const hasGit = computed(() => Boolean(props.gitStatus))
</script>

<template>
  <section class="frame" data-testid="frame">
    <header class="frame-header">
      <div class="frame-row frame-row-1">
        <span class="frame-profile-pill" :style="{ backgroundColor: phaseColor }">
          <span class="frame-profile-dot" :class="{ 'animate-pulse-slow': isPulsing }" aria-hidden="true" />
          {{ profile }}
        </span>
        <Pill mono color="var(--theme-fg-dim)">{{ modeTag }}</Pill>
        <Pill v-if="provider" mono color="var(--theme-fg-dim)" class="frame-provider-pill"
          >{{ provider }}<template v-if="model"> · {{ model }}</template></Pill
        >
        <span v-if="title" class="frame-title">{{ title }}</span>
        <span v-else class="frame-title-spacer" />
        <button type="button" class="frame-close" aria-label="close" @click="emit('close')">
          <FaIcon :icon="['fas', 'xmark']" class="frame-close-icon" />
        </button>
      </div>

      <div class="frame-row frame-row-2">
        <button type="button" class="frame-cwd" :aria-expanded="cwdExpanded" @click="emit('toggleCwd')">
          <FaIcon :icon="['fas', cwdExpanded ? 'chevron-down' : 'chevron-right']" class="frame-cwd-caret-icon" aria-hidden="true" />
          <span class="frame-cwd-label">cwd</span>
          <span v-if="cwd" class="frame-cwd-value">{{ cwd }}</span>
          <span v-if="hasGit" class="frame-cwd-git">
            <FaIcon :icon="['fas', 'code-branch']" class="frame-cwd-git-icon" aria-hidden="true" />
            <span class="frame-cwd-git-branch">{{ gitStatus!.branch }}</span>
            <span v-if="gitStatus!.ahead && gitStatus!.ahead > 0" class="frame-cwd-git-ahead">↑{{ gitStatus!.ahead }}</span>
            <span v-if="gitStatus!.behind && gitStatus!.behind > 0" class="frame-cwd-git-behind">↓{{ gitStatus!.behind }}</span>
          </span>
          <span v-if="hasGit && gitStatus!.worktree" class="frame-cwd-worktree">worktree: {{ gitStatus!.worktree }}</span>
        </button>
        <div class="frame-counts">
          <BreadcrumbPill v-for="c in counts" :key="c.label" :color="c.color" :label="c.label" :count="c.count" />
        </div>
      </div>

      <div v-if="$slots.toast" class="frame-toast">
        <slot name="toast" />
      </div>
    </header>

    <div class="frame-body">
      <slot />
    </div>

    <footer v-if="$slots.composer" class="frame-composer">
      <slot name="composer" />
    </footer>
  </section>
</template>

<style scoped>
@reference '../assets/styles.css';

.frame {
  @apply flex h-full min-h-0 flex-col;
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg);
  font-family: var(--theme-font-sans);
  /* The Frame is the overlay's width authority — every width-responsive
   * primitive inside it reads `@container frame (...)` so narrow anchors
   * (right/left 40% on small monitors) don't horizontal-scroll or clip. */
  container-type: inline-size;
  container-name: frame;
}

.frame-header {
  @apply flex flex-col border-b;
  border-color: var(--theme-border);
  background-color: var(--theme-surface);
}

.frame-row {
  @apply flex items-center gap-2 px-3 py-[6px];
}

.frame-row-2 {
  @apply border-t;
  border-color: var(--theme-border-soft);
}

.frame-profile-pill {
  @apply inline-flex shrink-0 items-center gap-[6px] rounded-sm px-[11px] py-[3px] text-[0.72rem] font-bold leading-tight;
  color: var(--theme-surface-bg);
  font-family: var(--theme-font-mono);
}

.frame-profile-dot {
  @apply inline-block h-[6px] w-[6px] shrink-0 rounded-full;
  background-color: var(--theme-surface-bg);
}

.frame-provider-pill {
  @apply shrink-0;
}

.frame-title {
  @apply ml-2 flex-1 truncate text-[0.8rem];
  min-width: 0;
  color: var(--theme-fg-ink-2);
}

.frame-title-spacer {
  @apply flex-1;
}

.frame-close {
  @apply shrink-0 border-0 bg-transparent px-1 text-[0.9rem] leading-none;
  color: var(--theme-fg-dim);
  cursor: pointer;
}

.frame-close-icon {
  width: 12px;
  height: 12px;
}

.frame-close:hover {
  color: var(--theme-status-err);
}

.frame-cwd {
  @apply inline-flex min-w-0 flex-1 items-center gap-1 border-0 bg-transparent px-1 text-[0.72rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
  cursor: pointer;
}

.frame-cwd-caret-icon {
  width: 10px;
  height: 10px;
  color: var(--theme-fg-faint);
}

.frame-cwd-label {
  @apply shrink-0;
  text-transform: lowercase;
}

.frame-cwd-value {
  @apply min-w-0 truncate;
  color: var(--theme-fg-ink-2);
}

.frame-cwd-git {
  @apply ml-auto inline-flex shrink-0 items-center gap-[4px] rounded-sm border px-[7px] py-[1px];
  background-color: var(--theme-surface-alt);
  border-color: var(--theme-border-soft);
}

.frame-cwd-git-icon {
  width: 9px;
  height: 9px;
  color: var(--theme-status-ok);
}

.frame-cwd-git-branch {
  @apply font-bold;
  color: var(--theme-status-ok);
}

.frame-cwd-git-ahead {
  color: var(--theme-state-stream);
}

.frame-cwd-git-behind {
  color: var(--theme-fg-dim);
}

.frame-cwd-worktree {
  @apply shrink-0 rounded-sm border px-[6px] py-[1px] text-[0.62rem];
  background-color: var(--theme-surface-alt);
  border-color: var(--theme-accent);
  color: var(--theme-accent);
}

.frame-counts {
  @apply ml-auto flex shrink-0 items-center gap-1;
}

.frame-toast {
  @apply px-3 py-[6px];
  border-top: 1px solid var(--theme-border-soft);
}

.frame-body {
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
}

.frame-composer {
  @apply border-t;
  border-color: var(--theme-border);
  background-color: var(--theme-surface);
}

/* Narrow-width rules. The provider/model pill is the first thing to drop
 * — title still communicates intent; provider/model is re-surfaced in the
 * palette. Git ahead/behind stays; only worktree hides below 340px because
 * its label is the longest chip on the row. */
@container frame (max-width: 420px) {
  .frame-provider-pill {
    display: none;
  }
}

@container frame (max-width: 340px) {
  .frame-cwd-worktree {
    display: none;
  }
}
</style>
