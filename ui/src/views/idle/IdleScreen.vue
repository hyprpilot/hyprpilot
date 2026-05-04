<script setup lang="ts">
/**
 * Idle landing — paints when the chat surface has no timeline content
 * (fresh daemon, no live turns yet). Centred wordmark + LFG. accent +
 * vertically aligned profile / adapter / model / cwd block + a live
 * sessions preview the captain can click to resume.
 *
 * Pure-presentational: receives every signal as a prop and emits
 * `restoreSession` for the click. The parent (Overlay.vue) owns
 * which sessions are surfaced and how many — this view just renders.
 */
import type { SessionSummary } from '@ipc'

defineProps<{
  profile?: string
  agent?: string
  model?: string
  cwd?: string
  sessions: SessionSummary[]
  totalSessionCount: number
}>()

const emit = defineEmits<{
  restoreSession: [sessionId: string]
}>()

function onRowClick(sessionId: string | undefined): void {
  if (!sessionId) {
    return
  }
  emit('restoreSession', sessionId)
}
</script>

<template>
  <section class="idle-screen" data-testid="idle-screen">
    <div class="idle-wordmark">hyprpilot</div>
    <div class="idle-accent">LFG.</div>
    <div class="idle-context" data-testid="idle-context">
      <span v-if="profile" class="idle-context-pill">
        <span class="idle-context-label">profile</span><span class="idle-context-value">{{ profile }}</span>
      </span>
      <span v-if="agent" class="idle-context-pill">
        <span class="idle-context-label">adapter</span><span class="idle-context-value">{{ agent }}</span>
      </span>
      <span v-if="model" class="idle-context-pill">
        <span class="idle-context-label">model</span><span class="idle-context-value">{{ model }}</span>
      </span>
      <span v-if="cwd" class="idle-context-pill">
        <span class="idle-context-label">cwd</span><span class="idle-context-value">{{ cwd }}</span>
      </span>
    </div>
    <div v-if="totalSessionCount > 0" class="idle-sessions">
      <header class="idle-sessions-header">
        <span class="idle-sessions-count">{{ totalSessionCount }}</span>
        <span class="idle-sessions-title">sessions</span>
        <span class="idle-sessions-line" />
      </header>
      <div class="idle-sessions-headrow">
        <span />
        <span>title</span>
        <span>cwd</span>
        <span>doing</span>
      </div>
      <div
        v-for="s in sessions"
        :key="s.sessionId"
        class="idle-sessions-row"
        :role="s.sessionId ? 'button' : undefined"
        :tabindex="s.sessionId ? 0 : undefined"
        :aria-label="s.sessionId ? `restore session ${s.title || s.sessionId}` : undefined"
        :data-restorable="Boolean(s.sessionId)"
        @click="onRowClick(s.sessionId)"
        @keydown.enter.prevent="onRowClick(s.sessionId)"
        @keydown.space.prevent="onRowClick(s.sessionId)"
      >
        <span class="idle-sessions-dot" aria-hidden="true">○</span>
        <span class="idle-sessions-cell">{{ s.title || s.sessionId }}</span>
        <span class="idle-sessions-cell idle-sessions-cwd">{{ s.cwd }}</span>
        <span class="idle-sessions-cell idle-sessions-doing">—</span>
      </div>
      <div v-if="totalSessionCount > sessions.length" class="idle-sessions-more">
        +{{ totalSessionCount - sessions.length }} more — Ctrl+K → sessions
      </div>
    </div>
    <div class="idle-kbd-hint">
      <kbd class="idle-kbd">Ctrl+K</kbd><span class="idle-kbd-label">command palette.</span>
    </div>
  </section>
</template>

<style scoped>
@reference '../../assets/styles.css';

.idle-screen {
  @apply flex flex-col items-center justify-center;
  flex: 1 1 auto;
  min-height: 100%;
  padding: 24px;
  color: var(--theme-fg-dim);
}

.idle-wordmark {
  font-family: var(--theme-font-mono);
  font-size: 26px;
  font-weight: 500;
  letter-spacing: -0.3px;
  color: var(--theme-fg);
}

.idle-accent {
  margin-top: 4px;
  font-family: var(--theme-font-mono);
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 1px;
  color: var(--theme-accent);
}

/* Context block below LFG — profile / adapter / model / cwd in a
 * vertically aligned stack. Two-column grid so the labels share a
 * right-aligned rail and the values share a left-aligned rail; the
 * eye reads down the labels first, then across to the data.
 * Hidden when nothing is configured (fresh daemon with no profile /
 * no agents). */
.idle-context {
  margin-top: 14px;
  display: grid;
  grid-template-columns: auto auto;
  justify-content: center;
  column-gap: 10px;
  row-gap: 4px;
  font-family: var(--theme-font-mono);
  font-size: 0.66rem;
  max-width: 100%;
}

.idle-context-pill {
  display: contents;
  white-space: nowrap;
}

.idle-context-label {
  /* "yellow for what it is" — the label name reads as the accent
   * key, mirroring the keybind-hint formatting captains already
   * grok from the footer. Right-aligned so the colon-rail lines
   * up across rows. */
  color: var(--theme-accent);
  font-weight: 600;
  text-align: right;
}

.idle-context-value {
  /* "white for the data" — the captured value reads as plain
   * default text against the accented label. Left-aligned so the
   * data column reads as one block. */
  color: var(--theme-fg);
  text-align: left;
}

.idle-sessions {
  width: 100%;
  max-width: 640px;
  margin-top: 26px;
}

.idle-sessions-header {
  @apply flex items-center;
  margin-bottom: 6px;
  font-family: var(--theme-font-mono);
  font-size: 0.62rem;
  color: var(--theme-fg-subtle);
  gap: 6px;
}

.idle-sessions-count {
  color: var(--theme-accent);
  font-weight: 700;
}

.idle-sessions-title {
  text-transform: lowercase;
}

.idle-sessions-line {
  flex: 1;
  height: 1px;
  background-color: var(--theme-border);
  margin-left: 8px;
}

.idle-sessions-headrow {
  display: grid;
  grid-template-columns: 14px 1fr 170px 110px;
  column-gap: 12px;
  padding: 4px 10px;
  font-family: var(--theme-font-mono);
  font-size: 0.56rem;
  color: var(--theme-fg-dim);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  border-bottom: 1px solid var(--theme-border);
}

.idle-sessions-row {
  display: grid;
  grid-template-columns: 14px 1fr 170px 110px;
  column-gap: 12px;
  align-items: center;
  padding: 7px 10px;
  border-bottom: 1px solid var(--theme-border);
  border-left: 2px solid var(--theme-status-ok);
  background-color: var(--theme-surface);
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
  color: var(--theme-fg);
  transition: background-color 0.12s ease-out;
}

.idle-sessions-row[data-restorable='true'] {
  cursor: pointer;
}

.idle-sessions-row[data-restorable='true']:hover,
.idle-sessions-row[data-restorable='true']:focus-visible {
  background-color: var(--theme-surface-alt);
  outline: 0;
}

.idle-sessions-dot {
  color: var(--theme-status-ok);
}

.idle-sessions-more {
  padding: 6px 10px;
  font-family: var(--theme-font-mono);
  font-size: 0.62rem;
  color: var(--theme-fg-dim);
  border-top: 1px solid var(--theme-border-soft);
  background-color: var(--theme-surface);
  letter-spacing: 0.4px;
}

.idle-sessions-cell {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--theme-fg);
}

.idle-sessions-cwd {
  color: var(--theme-fg-subtle);
}

.idle-sessions-doing {
  color: var(--theme-status-ok);
}

/* Sticky kbd hint at the bottom of the idle pane — surfaces the one
 * keybind the captain needs to discover everything else. Mirrors
 * the kbd-style formatting used in the chat footer hints. */
.idle-kbd-hint {
  margin-top: 22px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-family: var(--theme-font-mono);
  font-size: 0.62rem;
}

.idle-kbd {
  padding: 1px 6px;
  border-radius: 3px;
  border: 1px solid var(--theme-border-soft);
  background-color: var(--theme-surface);
  color: var(--theme-accent);
  font-weight: 600;
}

.idle-kbd-label {
  color: var(--theme-fg-dim);
}
</style>
