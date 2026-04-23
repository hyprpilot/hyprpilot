<script setup lang="ts">
import { computed } from 'vue'

import Button from '../Button.vue'
import { ButtonTone, ButtonVariant, type PermissionPrompt } from '../types'

/**
 * Permission prompt stack. Max-height 50vh, scrolls internally. The oldest
 * active (non-queued) row gets the solid `[allow once]` + ghost `[deny]`
 * buttons; every other row renders the command only with a "queued" label.
 * Top sticky banner shows the pending count.
 *
 * design-skip: the bundle originally mapped five option ids (allow-once /
 * allow-session / allow-always / deny-once / deny-always); chat1.md
 * reduced to two variants (`allow once` / `deny`), ported verbatim.
 */
const props = defineProps<{
  prompts: PermissionPrompt[]
}>()

const emit = defineEmits<{
  allow: [id: string]
  deny: [id: string]
}>()

// Oldest-first: the first non-queued prompt in the array gets action
// buttons. Everything else (queued OR later actives) renders as rows.
const activeId = computed(() => {
  const active = props.prompts.find((p) => !p.queued)
  return active?.id
})
</script>

<template>
  <section v-if="prompts.length > 0" class="permission-stack" data-testid="permission-stack">
    <header class="permission-stack-banner">
      <FaIcon :icon="['fas', 'triangle-exclamation']" class="permission-stack-banner-icon" aria-hidden="true" />
      <span class="permission-stack-count">{{ prompts.length }} pending</span>
      <span class="permission-stack-sep">· review oldest first · <FaIcon :icon="['fas', 'arrow-right-to-bracket']" class="permission-stack-skip-icon" /> to skip</span>
    </header>

    <ul class="permission-stack-list">
      <li v-for="(p, i) in prompts" :key="p.id" class="permission-stack-row" :data-active="p.id === activeId">
        <div class="permission-stack-meta">
          <span class="permission-stack-counter">{{ i + 1 }}/{{ prompts.length }}</span>
          <span class="permission-stack-tool">{{ p.tool }}</span>
          <span v-if="p.id === activeId" class="permission-stack-awaiting">· awaiting your approval</span>
          <code class="permission-stack-args">{{ p.args }}</code>
        </div>

        <span v-if="p.id === activeId" class="permission-stack-actions">
          <Button :variant="ButtonVariant.Solid" :tone="ButtonTone.Ok" @click="emit('allow', p.id)">allow once</Button>
          <Button :variant="ButtonVariant.Ghost" :tone="ButtonTone.Err" @click="emit('deny', p.id)">deny</Button>
        </span>
        <span v-else class="permission-stack-queued">queued</span>
      </li>
    </ul>
  </section>
</template>

<style scoped>
@reference '../../assets/styles.css';

.permission-stack {
  @apply flex flex-col overflow-hidden;
  border-top: 2px solid var(--theme-status-warn);
  background-color: var(--theme-permission-bg-active);
  max-height: 50vh;
}

.permission-stack-banner {
  @apply sticky top-0 z-10 flex items-center gap-2 px-3 py-[6px] text-[0.7rem];
  border-bottom: 1px solid var(--theme-border-soft);
  background-color: var(--theme-permission-bg-active);
  color: var(--theme-status-warn);
  font-family: var(--theme-font-mono);
}

.permission-stack-banner-icon {
  @apply inline-flex shrink-0 items-center justify-center rounded-sm;
  width: 18px;
  height: 18px;
  padding: 3px;
  background-color: var(--theme-status-warn);
  color: var(--theme-surface-bg);
}

.permission-stack-count {
  @apply font-bold;
}

.permission-stack-sep {
  color: var(--theme-fg-ink-2);
}

.permission-stack-skip-icon {
  width: 10px;
  height: 10px;
  vertical-align: -1px;
}

.permission-stack-list {
  @apply m-0 flex list-none flex-col gap-0 overflow-y-auto p-0;
}

.permission-stack-row {
  @apply flex flex-wrap items-center gap-2 border-b px-3 py-[10px] text-[0.78rem];
  border-color: var(--theme-border-soft);
  font-family: var(--theme-font-mono);
  background-color: var(--theme-permission-bg);
}

.permission-stack-row[data-active='true'] {
  background-color: var(--theme-permission-bg-active);
}

.permission-stack-row:last-child {
  border-bottom: none;
}

.permission-stack-meta {
  @apply flex min-w-0 flex-1 flex-col gap-1;
}

.permission-stack-counter {
  @apply mr-2 inline text-[0.7rem];
  color: var(--theme-fg-dim);
}

.permission-stack-tool {
  @apply inline font-bold;
  color: var(--theme-fg-ink-2);
}

.permission-stack-row[data-active='true'] .permission-stack-tool {
  color: var(--theme-state-awaiting);
}

.permission-stack-awaiting {
  @apply text-[0.7rem];
  color: var(--theme-fg-dim);
}

.permission-stack-args {
  @apply block truncate border-l-[3px] px-2 py-1 text-[0.78rem];
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  color: var(--theme-fg-ink-2);
  font-family: var(--theme-font-mono);
}

.permission-stack-row[data-active='true'] .permission-stack-args {
  border-left-color: var(--theme-state-awaiting);
  color: var(--theme-fg);
}

.permission-stack-actions {
  @apply ml-auto inline-flex shrink-0 items-center gap-1;
}

.permission-stack-queued {
  @apply ml-auto shrink-0 text-[0.7rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

/* Narrow widths: actions fall below the meta in their own full-width row.
 * Viewport media query (not container) because the stack renders inside
 * the Frame body — inheriting the Frame's container width would double-
 * match against `inline-size`. The overlay viewport is the right axis. */
@media (max-width: 420px) {
  .permission-stack-actions,
  .permission-stack-queued {
    @apply ml-0 w-full;
  }
}
</style>
