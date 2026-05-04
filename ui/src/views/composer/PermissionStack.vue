<script setup lang="ts">
import { computed } from 'vue'

import PermissionRow from './PermissionRow.vue'
import type { PermissionView } from '@components'

/**
 * Permission panel — pinned bottom band, max-height 45vh. Renders
 * every queued `PermissionView` whose formatter declared
 * `permissionUi: Row` (D7). Modal-class permissions are routed to
 * `<PermissionModal>` by the chat surface and never appear here.
 *
 * The panel only handles the inline-row case; the unified
 * `ToolCallView` from each formatter drives the row chrome via
 * `<PermissionRow>` — no MCP name parsing, no per-field projection,
 * no `kind`/`tool` decomposition lives at this layer.
 *
 * Multiple Row prompts process one at a time — only the oldest
 * non-queued prompt is fully interactive. The header counter
 * (rendered inside `PermissionRow`'s header when N > 1) shows
 * `current of total`.
 */
const props = defineProps<{
  views: PermissionView[]
}>()

const emit = defineEmits<{
  reply: [requestId: string, optionId: string]
  dismiss: [requestId: string]
}>()

const active = computed(() => props.views.find((v) => !v.queued))
const total = computed(() => props.views.length)
const activeIndex = computed(() => {
  if (!active.value) {
    return 0
  }

  return props.views.findIndex((v) => v.request.requestId === active.value!.request.requestId) + 1
})
</script>

<template>
  <section v-if="active" class="permission-panel" data-testid="permission-stack">
    <div v-if="total > 1" class="permission-panel-counter-strip">
      <span class="permission-panel-counter">{{ activeIndex }} of {{ total }}</span>
    </div>
    <PermissionRow
      :view="active"
      @reply="(optionId) => emit('reply', active!.request.requestId, optionId)"
      @dismiss="emit('dismiss', active!.request.requestId)"
    />
  </section>
</template>

<style scoped>
@reference '../../assets/styles.css';

.permission-panel {
  @apply flex flex-col overflow-y-auto;
  background-color: var(--theme-permission-bg);
  border-top: 2px solid var(--theme-status-warn);
  max-height: 45vh;
}

.permission-panel-counter-strip {
  @apply flex items-center;
  padding: 4px 14px;
  background-color: var(--theme-permission-bg);
  border-bottom: 1px solid var(--theme-border-soft);
}

.permission-panel-counter {
  @apply inline-flex shrink-0 items-center font-bold text-[0.6rem];
  background-color: var(--theme-surface-bg);
  border: 1px solid var(--theme-border-soft);
  color: var(--theme-fg);
  padding: 1px 7px;
  border-radius: 3px;
  letter-spacing: 0.4px;
  font-family: var(--theme-font-mono);
}
</style>
