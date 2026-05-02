<script setup lang="ts">
import { faCheck, faCheckDouble, faXmark } from '@fortawesome/free-solid-svg-icons'

import ToolSpecSheet from '../chat/ToolSpecSheet.vue'
import type { PermissionView } from '@components'

/**
 * Single permission row. Renders the unified `ToolCallView` chrome
 * (icon + composed title + structured fields) plus three action
 * buttons (allow once / allow always / deny). Deny is single-shot —
 * captain never asked for "deny always", and the trust store is
 * additive only (allow-list rather than deny-list).
 */
const props = defineProps<{
  view: PermissionView
}>()

const emit = defineEmits<{
  reply: [optionId: 'allow' | 'deny', remember: boolean]
  dismiss: []
}>()

void props
</script>

<template>
  <article class="permission-row" data-testid="permission-row">
    <header class="permission-row-header">
      <span class="permission-row-tool" :aria-label="view.call.title">
        <FaIcon :icon="view.call.icon" class="permission-row-icon" aria-hidden="true" />
        <span class="permission-row-title">{{ view.call.title }}</span>
      </span>
      <span class="permission-row-spacer" />
      <div class="permission-row-actions">
        <button type="button" class="permission-row-btn" data-tone="ok" data-variant="solid" aria-label="allow once" title="allow once" @click="emit('reply', 'allow', false)">
          <FaIcon :icon="faCheck" class="permission-row-btn-icon" aria-hidden="true" />
        </button>
        <button
          type="button"
          class="permission-row-btn"
          data-tone="ok"
          aria-label="allow always"
          title="allow always — remember this tool for the rest of the session"
          @click="emit('reply', 'allow', true)"
        >
          <FaIcon :icon="faCheckDouble" class="permission-row-btn-icon" aria-hidden="true" />
        </button>
        <button type="button" class="permission-row-btn" data-tone="err" aria-label="deny" title="deny" @click="emit('reply', 'deny', false)">
          <FaIcon :icon="faXmark" class="permission-row-btn-icon" aria-hidden="true" />
        </button>
      </div>
    </header>
    <div v-if="view.call.fields || view.call.description || view.call.output" class="permission-row-body">
      <ToolSpecSheet :description="view.call.description" :output="view.call.output" :fields="view.call.fields" />
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
  @apply sticky top-0 z-10 flex items-center gap-[10px] text-[0.7rem];
  background-color: var(--theme-permission-bg);
  border-bottom: 1px solid var(--theme-border-soft);
  font-family: var(--theme-font-mono);
  padding: 6px 14px 6px 4px;
}

.permission-row-tool {
  @apply inline-flex shrink-0 items-center gap-[5px] text-[0.62rem];
  background-color: var(--theme-status-warn);
  color: var(--theme-fg-on-tone);
  padding: 2px 7px;
  border-radius: 3px;
  font-weight: 700;
}

.permission-row-icon {
  width: 9px;
  height: 9px;
}

.permission-row-title {
  font-weight: 700;
}

.permission-row-spacer {
  flex: 1;
}

.permission-row-actions {
  @apply flex shrink-0 items-center gap-1;
}

.permission-row-btn {
  @apply inline-flex items-center justify-center;
  width: 22px;
  height: 22px;
  padding: 0;
  border-radius: 3px;
  background-color: transparent;
  cursor: pointer;
}

.permission-row-btn[data-tone='ok'] {
  border: 1px solid var(--theme-status-ok);
  color: var(--theme-status-ok);
}

.permission-row-btn[data-tone='err'] {
  border: 1px solid var(--theme-status-err);
  color: var(--theme-status-err);
}

.permission-row-btn[data-variant='solid'][data-tone='ok'] {
  background-color: var(--theme-status-ok);
  color: var(--theme-fg-on-tone);
}

.permission-row-btn-icon {
  width: 11px;
  height: 11px;
}

.permission-row-btn:hover {
  filter: brightness(1.15);
}

.permission-row-body {
  @apply flex flex-col;
  padding: 8px 10px;
}
</style>
