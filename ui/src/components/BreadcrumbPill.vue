<script setup lang="ts">
/**
 * Breadcrumb chip — wireframe BCPill. Plain rectangular pill, line2
 * border on all sides, mono ink2 text. `color` overrides the whole
 * text (used by `↻ resumed` to glow green).
 *
 * Two render modes:
 *  - prop-driven: `label` + `count` → renders `+{count} {label}` with
 *    the count in bold so the eye lands on the magnitude first.
 *  - slot-driven: pass children freely (e.g. `↻ resumed`).
 */
withDefaults(
  defineProps<{
    color?: string
    label?: string
    count?: number
  }>(),
  { color: 'var(--theme-fg-ink-2)' }
)
</script>

<template>
  <span class="breadcrumb-pill" :style="{ color }">
    <template v-if="count !== undefined && label">
      <span class="breadcrumb-pill-count">+{{ count }}</span>
      <span class="breadcrumb-pill-label">{{ label }}</span>
    </template>
    <template v-else-if="label">
      <span class="breadcrumb-pill-label">{{ label }}</span>
    </template>
    <slot />
  </span>
</template>

<style scoped>
@reference '../assets/styles.css';

.breadcrumb-pill {
  @apply inline-flex shrink-0 items-center gap-1 leading-tight;
  padding: 1px 7px;
  border: 1px solid var(--theme-border-soft);
  border-radius: 3px;
  background-color: var(--theme-surface);
  font-family: var(--theme-font-mono);
  font-size: 0.62rem;
}

.breadcrumb-pill-count {
  @apply font-bold;
}

.breadcrumb-pill-label {
  text-transform: lowercase;
}
</style>
