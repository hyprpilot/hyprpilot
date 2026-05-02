<script setup lang="ts">
/**
 * Right-pane preview for the instances palette leaf. Renders a quick
 * summary of the highlighted instance (captain-set name or profile
 * fallback, adapter + model) plus the last two transcript turns so
 * the captain can scan the recent conversation without focusing the
 * instance first.
 *
 * Reactive — pulls live state from `useTranscript(instanceId)` and
 * `useSessionInfo(instanceId)`. No fetch-on-focus / debounce since
 * these read from already-warmed in-memory composables.
 */
import { computed } from 'vue'

import { type PaletteEntry, useSessionInfo, useTranscript } from '@composables'
import type { InstanceListEntry } from '@ipc'

const props = defineProps<{
  entry?: PaletteEntry
  items: InstanceListEntry[]
}>()

const PREVIEW_TURN_COUNT = 2
const PREVIEW_BODY_CHAR_LIMIT = 360

const active = computed<InstanceListEntry | undefined>(() => {
  if (!props.entry) {
    return undefined
  }

  return props.items.find((i) => i.instanceId === props.entry?.id)
})

const headline = computed<string>(() => {
  const entry = active.value

  if (!entry) {
    return ''
  }

  return entry.name ?? entry.profileId ?? entry.agentId
})

const subhead = computed<string | undefined>(() => {
  const entry = active.value

  if (!entry) {
    return undefined
  }

  if (entry.name && entry.profileId) {
    return entry.profileId
  }

  return undefined
})

const sessionInfo = computed(() => {
  const id = active.value?.instanceId

  if (!id) {
    return undefined
  }
  const { info } = useSessionInfo(id)

  return info.value
})

const recentTurns = computed(() => {
  const id = active.value?.instanceId

  if (!id) {
    return []
  }
  const { turns } = useTranscript(id)

  return turns.value.slice(-PREVIEW_TURN_COUNT)
})

function preview(text: string): string {
  if (text.length <= PREVIEW_BODY_CHAR_LIMIT) {
    return text
  }

  return `${text.slice(0, PREVIEW_BODY_CHAR_LIMIT)}…`
}
</script>

<template>
  <div class="palette-instances-preview" data-testid="palette-instances-preview">
    <div v-if="!active" class="palette-instances-preview-empty" data-testid="palette-instances-preview-empty">no instance selected</div>
    <template v-else>
      <h3 class="palette-instances-preview-title">{{ headline }}</h3>
      <div v-if="subhead" class="palette-instances-preview-subhead">{{ subhead }}</div>
      <dl class="palette-instances-preview-meta">
        <div>
          <dt>adapter</dt>
          <dd>{{ active.agentId }}</dd>
        </div>
        <div v-if="sessionInfo?.model">
          <dt>model</dt>
          <dd>{{ sessionInfo.model }}</dd>
        </div>
        <div v-if="active.mode">
          <dt>mode</dt>
          <dd>{{ active.mode }}</dd>
        </div>
        <div>
          <dt>id</dt>
          <dd class="palette-instances-preview-id">{{ active.instanceId }}</dd>
        </div>
      </dl>

      <div v-if="recentTurns.length > 0" class="palette-instances-preview-turns">
        <div class="palette-instances-preview-turns-header">recent</div>
        <article v-for="turn in recentTurns" :key="turn.id" class="palette-instances-preview-turn" :data-role="turn.role">
          <header class="palette-instances-preview-turn-role">{{ turn.role }}</header>
          <p class="palette-instances-preview-turn-body">{{ preview(turn.text) }}</p>
        </article>
      </div>
      <div v-else class="palette-instances-preview-empty-turns">no turns yet</div>
    </template>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.palette-instances-preview {
  @apply flex flex-col gap-2 px-[14px] py-[12px];
}

.palette-instances-preview-empty,
.palette-instances-preview-empty-turns {
  @apply text-[0.72rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.palette-instances-preview-title {
  @apply m-0 text-left text-[0.9rem] font-semibold leading-tight;
  color: var(--theme-fg);
  letter-spacing: -0.1px;
}

.palette-instances-preview-subhead {
  @apply text-[0.65rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.palette-instances-preview-meta {
  @apply m-0 grid gap-y-[3px] gap-x-[10px] text-[0.7rem];
  font-family: var(--theme-font-mono);
  grid-template-columns: auto 1fr;
}

.palette-instances-preview-meta > div {
  @apply contents;
}

.palette-instances-preview-meta dt {
  color: var(--theme-fg-dim);
}

.palette-instances-preview-meta dd {
  @apply m-0 truncate;
  color: var(--theme-fg-ink-2);
}

.palette-instances-preview-id {
  @apply text-[0.6rem];
}

.palette-instances-preview-turns {
  @apply mt-1 flex flex-col gap-1;
}

.palette-instances-preview-turns-header {
  @apply text-[0.55rem] uppercase;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
  letter-spacing: 1px;
}

.palette-instances-preview-turn {
  @apply flex flex-col gap-[2px];
  padding: 5px 8px;
  border-radius: 3px;
  background-color: var(--theme-surface-bg);
  border-left: 2px solid var(--theme-border-soft);
}

.palette-instances-preview-turn[data-role='user'] {
  border-left-color: var(--theme-accent-user);
}

.palette-instances-preview-turn[data-role='agent'] {
  border-left-color: var(--theme-accent-assistant);
}

.palette-instances-preview-turn-role {
  @apply text-[0.55rem] uppercase;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
  letter-spacing: 1px;
}

.palette-instances-preview-turn-body {
  @apply m-0 text-[0.7rem] leading-snug;
  color: var(--theme-fg);
  white-space: pre-wrap;
  word-break: break-word;
  display: -webkit-box;
  -webkit-line-clamp: 4;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
</style>
