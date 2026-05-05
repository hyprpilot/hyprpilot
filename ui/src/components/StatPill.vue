<script setup lang="ts">
/**
 * Single bordered mini-pill for surfaces that surface metric chrome
 * (tool-call stats, turn elapsed, stream-card elapsed). Three knobs:
 *   - `label` — the text. Caller formats (`850ms`, `+12`, `2.4s`).
 *   - `tone`  — neutral (default), `ok` (green), `err` (red). Used by
 *               diff splits — `+N` is ok-toned, `−M` is err-toned.
 *   - `live`  — when true, pulsating dot + primary "working" colour
 *               so the captain reads the chip as still ticking.
 *
 * Replaces three pieces of bespoke chrome that drifted apart:
 * `ToolPillStats`'s inner pill, `Turn`'s `.turn-elapsed`, and
 * `StreamCard`'s `.stream-card-elapsed`. One component = one source
 * of truth for sizing / border / pulse.
 */
withDefaults(
  defineProps<{
    label: string
    tone?: 'neutral' | 'ok' | 'err'
    live?: boolean
  }>(),
  { tone: 'neutral', live: false }
)
</script>

<template>
  <span class="stat-pill" :data-tone="tone" :data-live="live">
    <span v-if="live" class="stat-pill-dot" aria-hidden="true" />
    <span>{{ label }}</span>
  </span>
</template>

<style scoped>
@reference '../assets/styles.css';

.stat-pill {
  @apply inline-flex shrink-0 items-center text-[0.56rem];
  font-family: var(--theme-font-mono);
  padding: 1px 5px;
  border: 1px solid var(--theme-border);
  border-radius: 3px;
  color: var(--theme-fg-dim);
  background-color: var(--theme-surface-bg);
  letter-spacing: 0.3px;
  line-height: 1.2;
  gap: 4px;
  /* Headers (`stream-card-header`, `tool-chips-header`) wrap the
   * pill in `text-transform: uppercase`, which would inherit and
   * render `13s` as `13S` / `1m 3s` as `1M 3S`. The pill is its
   * own typographic unit; pin lowercase so any host chrome can't
   * bleed casing in. */
  text-transform: none;
}

.stat-pill[data-tone='ok'] {
  color: var(--theme-status-ok);
  border-color: var(--theme-status-ok);
}

.stat-pill[data-tone='err'] {
  color: var(--theme-status-err);
  border-color: var(--theme-status-err);
}

/* Live = working: primary "stream" tone + a pulsating dot. Wins
 * over `data-tone` because a live diff would never make sense
 * (diffs settle by definition). */
.stat-pill[data-live='true'] {
  color: var(--theme-state-stream);
  border-color: var(--theme-state-stream);
}

.stat-pill-dot {
  @apply inline-block h-[4px] w-[4px] shrink-0 animate-pulse rounded-full;
  background-color: currentColor;
}
</style>
