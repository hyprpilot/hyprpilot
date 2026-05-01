<script setup lang="ts">
/**
 * Toast body for the cancelled / errored turn — displays the message
 * alongside an inline `delete` button that runs the parent's
 * `removeTurn` callback. Used by `use-session-stream`'s TurnEnded
 * handler. Lives as a dedicated SFC so the toast layer stays free
 * of inline `h(...)` markup, and the scoped CSS for the action
 * button stays component-local instead of leaking into the global
 * stylesheet.
 */
defineProps<{
  message: string
  /// Tone color CSS variable name (e.g. `var(--theme-status-warn)`).
  /// The action button outlines + fills with this colour so it
  /// reads as belonging to the surrounding toast level.
  tone: string
  /// Fired when the captain clicks `delete`. Removes the cancelled
  /// turn and its tools / stream from the transcript so the chat
  /// reads cleanly.
  onDelete: () => void
}>()
</script>

<template>
  <span class="cancel-toast-body">
    <span class="toast-message">{{ message }}</span>
    <button type="button" class="cancel-toast-action" :style="{ '--tone': tone }" @click="onDelete">delete</button>
  </span>
</template>

<style scoped>
@reference '../assets/styles.css';

.cancel-toast-body {
  @apply inline-flex flex-1 items-center;
  gap: 8px;
  min-width: 0;
}

.cancel-toast-action {
  @apply inline-flex shrink-0 items-center text-[0.6rem] font-bold uppercase;
  color: var(--tone);
  background-color: transparent;
  border: 1px solid var(--tone);
  border-radius: 3px;
  padding: 2px 8px;
  letter-spacing: 0.6px;
  cursor: pointer;
  font-family: var(--theme-font-mono);
}

.cancel-toast-action:hover {
  background-color: var(--tone);
  color: var(--theme-fg-on-tone);
}
</style>
