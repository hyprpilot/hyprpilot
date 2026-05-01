<script setup lang="ts">
import { faArrowRightArrowLeft } from '@fortawesome/free-solid-svg-icons'
import { computed } from 'vue'

/**
 * Chapter-break banner for "X changed from A to B" events. Generic
 * over the label (`mode`, `branch`, `cwd`, …); not tied to ACP
 * `current_mode_update`. Layout: horizontal-rule + label-pill +
 * horizontal-rule. The eye reads the rule pair as a chapter break,
 * not a card — distinct from a Turn body so the reader doesn't
 * mistake it for a message.
 *
 * Props are strings — caller resolves human names against whatever
 * registry it owns (mode list, branch list, profile list). Supplies
 * `from` only when the predecessor is known; falls back to
 * `→ <to>` on first emission.
 */
const props = defineProps<{
  /// Lead-in noun for the banner label, e.g. `mode`, `branch`, `cwd`.
  kind: string
  /// Resolved label of the new value. Required.
  to: string
  /// Resolved label of the prior value. Omitted on first emission.
  from?: string
}>()

const showFrom = computed(() => typeof props.from === 'string' && props.from.length > 0)
</script>

<template>
  <div
    class="change-banner"
    role="note"
    :aria-label="showFrom ? `${kind} changed from ${from} to ${to}` : `${kind} changed to ${to}`"
  >
    <span class="change-banner-rule" aria-hidden="true" />
    <span class="change-banner-label">
      <FaIcon :icon="faArrowRightArrowLeft" class="change-banner-icon" aria-hidden="true" />
      <span class="change-banner-text">
        <span class="change-banner-leader">{{ kind }}</span>
        <template v-if="showFrom">
          <span class="change-banner-sep" aria-hidden="true">·</span>
          <span class="change-banner-from">{{ from }}</span>
        </template>
        <span class="change-banner-arrow" aria-hidden="true">→</span>
        <strong class="change-banner-to">{{ to }}</strong>
      </span>
    </span>
    <span class="change-banner-rule" aria-hidden="true" />
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.change-banner {
  @apply flex items-center;
  margin: 6px 0;
  gap: 8px;
  font-family: var(--theme-font-mono);
  font-size: 0.62rem;
  letter-spacing: 0.6px;
  color: var(--theme-fg-dim);
  text-transform: uppercase;
}

.change-banner-rule {
  flex: 1 1 auto;
  height: 1px;
  background-color: var(--theme-border-soft);
}

.change-banner-label {
  @apply inline-flex items-center;
  gap: 6px;
  padding: 2px 8px;
  border: 1px solid var(--theme-border-soft);
  border-radius: 2px;
  background-color: var(--theme-surface);
  color: var(--theme-fg);
}

.change-banner-text {
  @apply inline-flex items-center;
  gap: 5px;
}

.change-banner-leader {
  color: var(--theme-fg-dim);
}

.change-banner-sep {
  color: var(--theme-fg-faint);
}

.change-banner-from {
  color: var(--theme-fg-ink-2);
  font-weight: 600;
}

.change-banner-arrow {
  color: var(--theme-fg-dim);
}

.change-banner-to {
  color: var(--theme-accent);
  font-weight: 700;
}

.change-banner-icon {
  width: 8px;
  height: 8px;
  color: var(--theme-accent);
}
</style>
