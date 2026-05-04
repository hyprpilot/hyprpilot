<script setup lang="ts">
/**
 * Right-pane preview for `openSessionsLeaf()`. Bound by
 * `CommandPalette.vue` via `top.preview.component` and receives the
 * currently highlighted entry as a prop. Calls `sessions_info` (via
 * the `getSessionInfo` IPC bridge) on focus change with a 200ms
 * debounce so arrow-key sweeps don't flood the daemon.
 *
 * Empty state when no row is highlighted (filter yielded zero rows
 * or the session list itself is empty); loading state while the
 * info round-trip is in flight; error state when the daemon rejects
 * (stale id, list not available).
 */
import { ref, watch } from 'vue'

import { Loading } from '@components'
import { type PaletteEntry } from '@composables'
import { useHomeDir } from '@composables'
import { invoke, TauriCommand, type SessionInfoResult } from '@ipc'

const props = defineProps<{
  entry?: PaletteEntry
}>()

const { displayPath } = useHomeDir()

const info = ref<SessionInfoResult>()
const loading = ref(false)
const lastErr = ref<string>()

let pendingTimer: ReturnType<typeof setTimeout> | undefined

function clearPending(): void {
  if (pendingTimer !== undefined) {
    clearTimeout(pendingTimer)
    pendingTimer = undefined
  }
}

watch(
  () => props.entry?.id,
  (id) => {
    clearPending()

    if (!id) {
      info.value = undefined
      loading.value = false
      lastErr.value = undefined

      return
    }
    // Last-write-wins guard: only the latest scheduled request should
    // commit its result. Captured by id snapshot at fetch time.
    const targetId = id

    pendingTimer = setTimeout(() => {
      pendingTimer = undefined
      loading.value = true
      lastErr.value = undefined
      void invoke(TauriCommand.SessionsInfo, { id: targetId })
        .then((result) => {
          if (props.entry?.id !== targetId) {
            return
          }
          info.value = result
          loading.value = false
        })
        .catch((err) => {
          if (props.entry?.id !== targetId) {
            return
          }
          lastErr.value = String(err)
          info.value = undefined
          loading.value = false
        })
    }, 200)
  },
  { immediate: true }
)

function formatCwd(raw: string): string {
  return displayPath(raw)
}
</script>

<template>
  <div class="palette-sessions-preview" data-testid="palette-sessions-preview">
    <div v-if="!entry" class="palette-sessions-preview-state-msg" data-testid="palette-sessions-preview-empty">no session selected</div>
    <Loading v-else-if="loading" mode="inline" status="loading session info" data-testid="palette-sessions-preview-loading" />
    <div v-else-if="lastErr" class="palette-sessions-preview-state-msg palette-sessions-preview-err" data-testid="palette-sessions-preview-err">
      {{ lastErr }}
    </div>
    <template v-else-if="info">
      <h3 class="palette-sessions-preview-title">{{ info.title || info.id }}</h3>
      <dl class="palette-sessions-preview-meta">
        <div>
          <dt>id</dt>
          <dd class="text-[0.65rem] font-mono">{{ info.id }}</dd>
        </div>
        <div>
          <dt>cwd</dt>
          <dd>{{ formatCwd(info.cwd) }}</dd>
        </div>
        <div v-if="info.lastTurnAt">
          <dt>last turn</dt>
          <dd>{{ info.lastTurnAt }}</dd>
        </div>
        <div>
          <dt>agent</dt>
          <dd>{{ info.agentId }}</dd>
        </div>
        <div v-if="info.profileId">
          <dt>profile</dt>
          <dd>{{ info.profileId }}</dd>
        </div>
      </dl>
    </template>
  </div>
</template>

<style scoped>
@reference '../assets/styles.css';

.palette-sessions-preview {
  @apply flex flex-col gap-1 px-[14px] py-[12px];
}

.palette-sessions-preview-state-msg {
  @apply text-[0.72rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.palette-sessions-preview-err {
  color: var(--theme-status-err);
}

.palette-sessions-preview-title {
  @apply m-0 text-left text-[0.9rem] font-semibold leading-tight;
  color: var(--theme-fg);
  letter-spacing: -0.1px;
}

.palette-sessions-preview-meta {
  @apply m-0 mt-[6px] grid gap-y-[3px] gap-x-[10px] text-[0.7rem];
  font-family: var(--theme-font-mono);
  grid-template-columns: auto 1fr;
}

.palette-sessions-preview-meta > div {
  @apply contents;
}

.palette-sessions-preview-meta dt {
  color: var(--theme-fg-dim);
}

.palette-sessions-preview-meta dd {
  @apply m-0 truncate;
  color: var(--theme-fg-subtle);
}
</style>
