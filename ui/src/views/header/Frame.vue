<script setup lang="ts">
import { faArrowDown, faArrowUp, faXmark } from '@fortawesome/free-solid-svg-icons'
import { computed } from 'vue'

import { BreadcrumbPill, Phase, phaseToCssSuffix, type BreadcrumbCount, type GitStatus } from '@components'

/**
 * Overlay chrome per wireframe Frame template:
 *
 *   row 1  [phase profile pill (·dot pulses when active)]
 *          [provider/model] [mode pill?] [session title?]   [✕]
 *   row 2  [cwd · git ↑↓ status (inline, accent left stripe)]
 *          [+22 mcps] [+4 skills]  (counts via BreadcrumbPill)
 *   body   absolute toast card (slot) + caller children
 *
 * 3px left phase-color border on the outer container (visual law #1).
 * Profile pill bg + the border share the phase color exactly. Mode +
 * title pills hide when their data is missing — same omit-when-empty
 * rule, no fabricated placeholder copy.
 */
const props = withDefaults(
  defineProps<{
    profile: string
    /// Captain-set instance name. When present the leftmost row-1
    /// pill renders this (the captain's own slug); profile pill
    /// shifts to its right. When `undefined` the profile pill stays
    /// leftmost — same legacy shape.
    name?: string
    phase?: Phase
    modeTag?: string
    provider?: string
    model?: string
    title?: string
    cwd?: string
    cwdFull?: string
    gitStatus?: GitStatus
    counts?: BreadcrumbCount[]
    cwdExpanded?: boolean
    /// Total live-instance count. Renders the row-1 instances pill
    /// when ≥ 2 (one means "just this", no extras to surface).
    instancesCount?: number
  }>(),
  {
    phase: Phase.Idle,
    counts: () => [],
    cwdExpanded: false,
    instancesCount: 0
  }
)

const emit = defineEmits<{
  close: []
  toggleCwd: []
  /// Emitted when the user clicks a row-1 pill (`profile` / `mode` /
  /// `provider`); the parent dispatches the matching palette leaf.
  pillClick: [target: 'profile' | 'mode' | 'provider']
  /// Emitted when the user clicks a breadcrumb pill in row 2; the
  /// parent dispatches the matching palette leaf. Pill id falls back
  /// to `label` when `BreadcrumbCount.id` is unset.
  breadcrumbClick: [id: string]
  /// Emitted when the captain clicks the row-1 instances pill — the
  /// parent opens the instances palette.
  instancesClick: []
}>()

const phaseColor = computed(() => `var(--theme-state-${phaseToCssSuffix(props.phase)})`)
const isPulsing = computed(() => props.phase === Phase.Streaming || props.phase === Phase.Working || props.phase === Phase.Awaiting || props.phase === Phase.Pending)
const hasGit = computed(() => Boolean(props.gitStatus))
</script>

<template>
  <section class="frame" :style="{ '--frame-phase': phaseColor }" data-testid="frame">
    <header class="frame-header">
      <div class="frame-row frame-row-1">
        <!-- Captain-set name takes the leftmost slot when present:
             the dot+phase color stays here (it's the active-instance
             marker). Profile pill becomes a secondary breadcrumb to
             its right. When no name is set, the profile pill keeps
             the dot — legacy shape. -->
        <button v-if="name" type="button" class="frame-profile-pill" :style="{ backgroundColor: phaseColor }" aria-label="instance name" @click="emit('pillClick', 'profile')">
          <span class="frame-profile-dot" :class="{ 'animate-pulse': isPulsing }" aria-hidden="true" />
          {{ name }}
        </button>
        <button
          type="button"
          class="frame-profile-pill"
          :style="!name ? { backgroundColor: phaseColor } : undefined"
          :data-secondary="Boolean(name)"
          aria-label="profile"
          @click="emit('pillClick', 'profile')"
        >
          <span v-if="!name" class="frame-profile-dot" :class="{ 'animate-pulse': isPulsing }" aria-hidden="true" />
          {{ profile }}
        </button>
        <button v-if="provider" type="button" class="frame-adapter-pill" aria-label="adapter" @click="emit('pillClick', 'provider')">
          {{ provider }}
        </button>
        <button v-if="model" type="button" class="frame-model-pill" aria-label="model" @click="emit('pillClick', 'provider')">
          {{ model }}
        </button>
        <button v-if="modeTag" type="button" class="frame-mode-pill" aria-label="mode" @click="emit('pillClick', 'mode')">
          {{ modeTag }}
        </button>
        <span v-if="title" class="frame-title">{{ title }}</span>
        <span v-else class="frame-title-spacer" />
        <button
          v-if="instancesCount > 1"
          type="button"
          class="frame-instances-pill"
          :aria-label="`${instancesCount} instances`"
          @click="emit('instancesClick')"
        >
          <span class="frame-instances-count">{{ instancesCount }}</span>
          <span class="frame-instances-label">instances</span>
        </button>
        <button type="button" class="frame-close" aria-label="close" @click="emit('close')">
          <FaIcon :icon="faXmark" class="frame-close-icon" />
        </button>
      </div>

      <div class="frame-row frame-row-2">
        <button type="button" class="frame-cwd" :aria-expanded="cwdExpanded" :title="cwdFull ?? cwd" @click="emit('toggleCwd')">
          <span v-if="cwd" class="frame-cwd-value">{{ cwd }}</span>
          <span v-else class="frame-cwd-value frame-cwd-value-empty">—</span>
          <span v-if="hasGit" class="frame-cwd-git">
            <span class="frame-cwd-git-branch">{{ gitStatus!.branch }}</span>
            <span v-if="gitStatus!.ahead && gitStatus!.ahead > 0" class="frame-cwd-git-ahead">
              <FaIcon :icon="faArrowUp" class="frame-cwd-git-arrow" aria-hidden="true" />{{ gitStatus!.ahead }}
            </span>
            <span class="frame-cwd-git-behind"> <FaIcon :icon="faArrowDown" class="frame-cwd-git-arrow" aria-hidden="true" />{{ gitStatus!.behind ?? 0 }} </span>
          </span>
        </button>
        <div class="frame-counts">
          <button v-for="c in counts" :key="c.label" type="button" class="frame-pill-button" :aria-label="c.id ?? c.label" @click="emit('breadcrumbClick', c.id ?? c.label)">
            <BreadcrumbPill :color="c.color" :label="c.label" :count="c.count" />
          </button>
        </div>
      </div>
    </header>

    <div class="frame-body">
      <!-- Toast slot — wireframe spec puts the toast card absolutely
           positioned over the chat body, "out of the header into the
           chat window", NOT a viewport portal. Parent passes the
           active toast (if any) into this slot. -->
      <slot name="toast" />
      <slot />
    </div>

    <footer v-if="$slots.composer" class="frame-composer">
      <slot name="composer" />
    </footer>
  </section>
</template>

<style scoped>
@reference '../../assets/styles.css';

.frame {
  @apply flex h-full min-h-0 flex-col;
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg);
  font-family: var(--theme-font-sans);
  /* visual law #1 — phase color as the instance border. Replaces the
   * old `--theme-window-edge` body border so we have one source of
   * "what state is this overlay in" rather than two stacked stripes.
   * Edge selection: in anchor mode the stripe paints the inward
   * edge (opposite the anchored side, the one users actually see);
   * in center mode the whole perimeter glows. The reactive color
   * rides on `--frame-phase` (set inline from the `phase` prop). */
  --frame-phase: var(--theme-state-idle);
  /* The Frame is the overlay's width authority — every width-responsive
   * primitive inside it reads `@container frame (...)` so narrow anchors
   * (right/left 40% on small monitors) don't horizontal-scroll or clip. */
  container-type: inline-size;
  container-name: frame;
}

html[data-window-anchor='right'] .frame {
  border-left: 3px solid var(--frame-phase);
}

html[data-window-anchor='left'] .frame {
  border-right: 3px solid var(--frame-phase);
}

html[data-window-anchor='top'] .frame {
  border-bottom: 3px solid var(--frame-phase);
}

html[data-window-anchor='bottom'] .frame {
  border-top: 3px solid var(--frame-phase);
}

html:not([data-window-anchor]) .frame {
  border: 3px solid var(--frame-phase);
}

.frame-header {
  @apply flex flex-col;
  background-color: var(--theme-surface);
}

/* Row 1 — wireframe spec: padding 8px 14px 8px 4px (asymmetric, 4px
 * left because the 3px phase border already lives outside the section).
 * Gap 10px between row items. Each row owns its own border-bottom so
 * the divider stays attached to the row when one is hidden. */
.frame-row-1 {
  @apply flex items-center border-b;
  padding: 8px 14px 8px 4px;
  gap: 10px;
  border-color: var(--theme-border);
}

.frame-row-2 {
  @apply flex items-stretch border-b;
  padding: 5px 14px 5px 4px;
  gap: 6px;
  background-color: var(--theme-surface-bg);
  border-color: var(--theme-border);
}

/* Profile pill — phase color bg + dark ink + mono. Pulse dot when the
 * session is in any active phase (working/streaming/awaiting/pending). */
.frame-profile-pill {
  @apply inline-flex shrink-0 items-center gap-[6px] rounded-sm border-0 leading-tight;
  padding: 3px 11px;
  font-size: 0.7rem;
  font-weight: 700;
  color: var(--theme-fg-on-tone);
  font-family: var(--theme-font-mono);
  cursor: pointer;
}

/* Secondary profile pill (rendered to the right of a captain-set
 * name): drops the phase fill, becomes a quieter breadcrumb so the
 * captain's slug stays the visual anchor. */
.frame-profile-pill[data-secondary='true'] {
  background-color: var(--theme-surface);
  color: var(--theme-accent);
  border: 1px solid var(--theme-border-soft);
}

.frame-profile-dot {
  @apply inline-block h-[6px] w-[6px] shrink-0 rounded-full;
  background-color: var(--theme-fg-on-tone);
}

/* Adapter / model / mode pills share the same chrome — surface fill,
 * soft border, accent-coloured text. Each one's accent is sourced
 * from a distinct token so the captain can tell at a glance which
 * pill they're reading without scanning the prefix:
 *   adapter  → kind-acp (light blue, the protocol surface)
 *   model    → kind-agent (purple, the LLM identity)
 *   mode     → accent (yellow, the operational lever) */
.frame-adapter-pill,
.frame-model-pill,
.frame-mode-pill {
  @apply inline-flex shrink-0 items-center rounded-sm leading-tight;
  padding: 3px 9px;
  font-size: 0.66rem;
  font-weight: 700;
  background-color: var(--theme-surface-alt);
  border: 1px solid var(--theme-border-soft);
  font-family: var(--theme-font-mono);
  cursor: pointer;
}

.frame-adapter-pill {
  color: var(--theme-kind-acp);
}

.frame-model-pill {
  color: var(--theme-kind-agent);
}

.frame-mode-pill {
  color: var(--theme-accent);
}

.frame-pill-button {
  @apply inline-flex shrink-0 items-center border-0 bg-transparent p-0;
  cursor: pointer;
}

/* Session title — italic dashed pill, mono, ink-2, ellipsizes. */
.frame-title {
  @apply flex-1 truncate;
  padding: 3px 10px;
  font-size: 0.66rem;
  min-width: 0;
  color: var(--theme-fg-subtle);
  background-color: var(--theme-surface-bg);
  border: 1px dashed var(--theme-border-soft);
  border-radius: 4px;
  font-family: var(--theme-font-mono);
  font-style: italic;
}

.frame-title-spacer {
  @apply flex-1;
}

/* Row 1 instances pill — sits between the title (or spacer) and the
 * close button; queue-style format (small accent-soft fill, accent
 * fg) so the captain reads "+N" first. Hidden when the registry has
 * a single instance: nothing extra to surface. */
.frame-instances-pill {
  @apply inline-flex shrink-0 cursor-pointer items-center gap-[5px] border-0 leading-tight;
  padding: 2px 8px;
  font-size: 0.6rem;
  font-weight: 700;
  color: var(--theme-accent);
  background-color: color-mix(in srgb, var(--theme-accent) 18%, transparent);
  border-radius: 3px;
  font-family: var(--theme-font-mono);
}

.frame-instances-pill:hover {
  filter: brightness(1.1);
}

.frame-instances-count {
  font-weight: 700;
}

.frame-instances-label {
  font-weight: 500;
  text-transform: lowercase;
  letter-spacing: 0.3px;
}

.frame-cwd-git-arrow {
  width: 7px;
  height: 7px;
  margin-right: 1px;
}

.frame-close {
  @apply shrink-0 border-0 bg-transparent px-1 leading-none;
  font-size: 0.9rem;
  color: var(--theme-fg-dim);
  cursor: pointer;
  font-family: var(--theme-font-mono);
}

.frame-close-icon {
  width: 12px;
  height: 12px;
}

.frame-close:hover {
  color: var(--theme-status-err);
}

/* Row 2 — cwd pill on the left (flex:1, accent yellow left-stripe,
 * surface fill, embedded git status pill on the right). Counts (mcps
 * / skills / etc.) sit alongside as breadcrumb pills. */
.frame-cwd {
  @apply inline-flex min-w-0 flex-1 items-center;
  padding: 3px 10px;
  gap: 6px;
  font-size: 0.66rem;
  color: var(--theme-fg);
  background-color: var(--theme-surface);
  border: 1px solid var(--theme-border-soft);
  border-left: 3px solid var(--theme-accent);
  border-radius: 3px;
  font-family: var(--theme-font-mono);
  cursor: pointer;
  overflow: hidden;
}

.frame-cwd-value {
  @apply min-w-0 truncate;
  color: var(--theme-fg);
}

.frame-cwd-value-empty {
  color: var(--theme-fg-dim);
}

.frame-cwd-git {
  @apply ml-auto inline-flex shrink-0 items-center;
  padding: 1px 7px;
  gap: 6px;
  border-radius: 3px;
  background-color: var(--theme-surface-alt);
  border: 1px solid var(--theme-border-soft);
}

.frame-cwd-git-branch {
  font-weight: 700;
  color: var(--theme-status-ok);
}

.frame-cwd-git-ahead {
  color: var(--theme-state-stream);
}

.frame-cwd-git-behind {
  color: var(--theme-fg-dim);
}

.frame-counts {
  @apply flex shrink-0 items-center gap-1;
}

/* Body is a positioning context for the absolute-positioned toast
 * card. `min-height: 0` is the standard flexbox-overflow guard so
 * the chat-transcript inside can scroll instead of pushing the
 * frame past viewport. */
.frame-body {
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
  position: relative;
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
