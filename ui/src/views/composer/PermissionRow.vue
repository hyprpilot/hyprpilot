<script setup lang="ts">
import ToolBody from '../chat/ToolBody.vue'
import { PermissionActions, ToastTone, ToolHeader } from '@components'
import type { PermissionView } from '@components'

/**
 * Single permission row. Header chrome comes from `<ToolHeader>` so it
 * stays consistent with `PermissionModal` and `ToolPill`. Action button
 * row comes from `<PermissionActions>` so the row + modal share the
 * same equal-width / ellipsis-truncating button layout.
 *
 * Emits `reply` with the real `optionId` from the offered set. Hyprpilot
 * is transparent to the agent's permission semantics — the captain's
 * pick rides the wire as-is.
 */
defineProps<{
  view: PermissionView
}>()

const emit = defineEmits<{
  reply: [optionId: string]
  dismiss: []
}>()
</script>

<template>
  <article class="permission-row" data-testid="permission-row">
    <!-- Two-row header. Long tool titles used to crowd the action
         buttons via the previous inline `#trailing` slot — narrow
         widths pushed the buttons off-screen or clipped the title
         to nothing. Splitting title + actions into their own rows
         gives each one the full width: the title pill grows to span
         the row and ellipsises only when the content actually
         overflows; the action buttons keep their existing equal-
         share layout below. -->
    <ToolHeader class="permission-row-header" :icon="view.call.icon" :title="view.call.title" :tone="ToastTone.Warn" />
    <PermissionActions class="permission-row-actions" :options="view.options" :default-option-id="view.defaultOptionId" @reply="(id) => emit('reply', id)" />
    <div class="permission-row-body">
      <ToolBody :view="view.call" />
    </div>
  </article>
</template>

<style scoped>
@reference '../../assets/styles.css';

.permission-row {
  @apply flex flex-col;
  background-color: var(--theme-permission-bg);
  border-top: 1px solid var(--theme-border-soft);
}

.permission-row-header {
  @apply sticky top-0 z-10 text-[0.7rem];
  background-color: var(--theme-permission-bg);
  padding: 0.375rem 0.875rem 0.375rem 0.25rem;
}

.permission-row-actions {
  /* Second row of the split header — full-width action cluster.
   * The previous `flex: 0 1 60%` cap was needed only when the
   * actions shared the row with the title; with the rows split
   * the actions own the full width and equal-share among
   * themselves via `PermissionActions`'s internal layout. The
   * bottom border lives on this row so the header / actions pair
   * visually reads as one chrome unit before the body padding. */
  padding: 0 0.875rem 0.375rem 0.25rem;
  border-bottom: 1px solid var(--theme-border-soft);
}

.permission-row-body {
  @apply flex flex-col;
  padding: 0.5rem 0.625rem;
}
</style>
