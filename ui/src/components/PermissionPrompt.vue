<script setup lang="ts">
import type { PermissionRequestEvent } from '@composables'

defineProps<{
  request?: PermissionRequestEvent
}>()
</script>

<template>
  <aside v-if="request" class="permission-prompt" data-testid="permission-prompt">
    <header class="permission-prompt-header">
      <span class="permission-prompt-dot" aria-hidden="true" />
      <h2 class="permission-prompt-title">permission auto-denied</h2>
    </header>

    <p class="permission-prompt-body">
      Session <code>{{ request.session_id }}</code> requested permission to run a tool. Auto-denied until the PermissionController lands.
    </p>

    <ul class="permission-prompt-options">
      <li v-for="opt in request.options" :key="opt.option_id">
        <code>{{ opt.option_id }}</code> — {{ opt.name }} ({{ opt.kind }})
      </li>
    </ul>
  </aside>
</template>

<style scoped>
@reference "../assets/styles.css";

.permission-prompt {
  @apply mt-4 flex flex-col gap-2 border px-4 py-3 text-[0.85rem];
  background-color: var(--theme-surface-compose);
  color: var(--theme-fg);
  border-color: var(--theme-border-focus);
}

.permission-prompt-header {
  @apply flex items-center gap-2;
}

.permission-prompt-dot {
  @apply h-2 w-2 rounded-full;
  background-color: var(--theme-state-awaiting);
}

.permission-prompt-title {
  @apply text-[0.9rem] font-bold tracking-wider;
  color: var(--theme-fg);
}

.permission-prompt-body {
  @apply leading-snug;
  color: var(--theme-fg-dim);
}

.permission-prompt-options {
  @apply m-0 flex list-none flex-col gap-1 p-0;
  color: var(--theme-fg-muted);
}
</style>
