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
    <ToolHeader class="permission-row-header" :icon="view.call.icon" :title="view.call.title" :tone="ToastTone.Warn">
      <template #trailing>
        <PermissionActions class="permission-row-actions" :options="view.options" @reply="(id) => emit('reply', id)" />
      </template>
    </ToolHeader>
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
  border-bottom: 1px solid var(--theme-border-soft);
  padding: 6px 14px 6px 4px;
}

.permission-row-actions {
  /* Constrain the action row's share of the header's available width.
   * Without a cap a long agent label can crowd the title pill; the
   * percentage keeps actions in their own gutter while still allowing
   * equal-width sharing inside that gutter. */
  flex: 0 1 60%;
  justify-content: flex-end;
}

.permission-row-body {
  @apply flex flex-col;
  padding: 8px 10px;
}
</style>
