<script setup lang="ts">
import { computed, ref, useSlots, watch } from 'vue'

import { Role } from '../types'
import { log, renderMarkdown } from '@lib'

/**
 * Assistant text card. Default behaviour preserves the slot — both
 * roles render the slot's text content into a styled lane, so
 * `<ChatBody :role="Role.User">{{ text }}</ChatBody>` keeps working.
 *
 * Setting `:markdown` + `:text` switches the assistant lane to the
 * markdown pipeline (markdown-it + Shiki + DOMPurify). User-role text
 * stays raw (markdown formatting in user input is the user's literal
 * intent and should not be re-rendered). The user-role pre-wrap lane
 * still uses the slot fallback so `:text` is ignored there.
 */
const props = defineProps<{
  role: Role
  /** Optional markdown source. Required when `markdown` is true. */
  text?: string
  /** Render `text` through the markdown pipeline (assistant role only). */
  markdown?: boolean
}>()

const accent = computed(() => (props.role === Role.User ? 'var(--theme-accent-user)' : 'var(--theme-accent-assistant)'))

const renderedHtml = ref('')
const useMarkdown = computed(() => props.markdown === true && props.role === Role.Assistant)
const slots = useSlots()
const slotEmpty = computed(() => !slots.default)

watch(
  [() => props.text, useMarkdown],
  async ([raw, on]) => {
    if (!on || !raw) {
      renderedHtml.value = ''

      return
    }
    try {
      const out = await renderMarkdown(raw)
      renderedHtml.value = out.html
    } catch (err) {
      log.warn('markdown render failed; falling back to plain text', { err: String(err) })
      renderedHtml.value = ''
    }
  },
  { immediate: true }
)

function onCopyClick(event: MouseEvent): void {
  const target = event.target as HTMLElement | null
  const button = target?.closest('button[data-md-copy]') as HTMLButtonElement | null
  if (!button) {
    return
  }
  const block = button.closest('.md-codeblock')
  const code = block?.querySelector('pre code')?.textContent ?? ''
  if (!code) {
    return
  }
  void navigator.clipboard
    .writeText(code)
    .then(() => {
      button.dataset.copied = 'true'
      window.setTimeout(() => {
        delete button.dataset.copied
      }, 1200)
    })
    .catch((err) => {
      log.warn('copy failed', { err: String(err) })
    })
}
</script>

<template>
  <div class="chat-body" :data-role="role" :style="{ '--accent': accent }">
    <!-- eslint-disable-next-line vue/no-v-html -- HTML is sanitised by renderMarkdown -->
    <div v-if="useMarkdown && renderedHtml" class="chat-body-md prose" v-html="renderedHtml" @click="onCopyClick" />
    <div v-else-if="useMarkdown && !renderedHtml && text" class="chat-body-plain">{{ text }}</div>
    <slot v-else-if="!slotEmpty" />
    <div v-else-if="text" class="chat-body-plain">{{ text }}</div>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.chat-body {
  @apply border-l-[3px] px-3 py-2 text-[0.9rem] leading-snug;
  color: var(--theme-fg);
  background-color: var(--theme-surface);
  border-color: var(--accent);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  overflow-wrap: anywhere;
  min-width: 0;
  font-family: var(--theme-font-sans);
}

.chat-body[data-role='user'] {
  white-space: pre-wrap;
}

.chat-body-plain {
  white-space: pre-wrap;
}

.chat-body-md :deep(p) {
  @apply my-1;
}

.chat-body-md :deep(p:first-child) {
  @apply mt-0;
}

.chat-body-md :deep(p:last-child) {
  @apply mb-0;
}

.chat-body-md :deep(ul),
.chat-body-md :deep(ol) {
  @apply my-1 pl-5;
}

.chat-body-md :deep(li) {
  @apply my-0.5;
}

.chat-body-md :deep(blockquote) {
  @apply my-1 border-l-2 pl-2;
  border-color: var(--theme-border);
  color: var(--theme-fg-dim);
}

.chat-body-md :deep(a) {
  color: var(--theme-accent);
  text-decoration: underline;
  text-underline-offset: 2px;
}

.chat-body-md :deep(code) {
  @apply rounded-sm px-1 py-[1px] text-[0.85em];
  font-family: var(--theme-font-mono);
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
}

.chat-body-md :deep(.md-codeblock) {
  @apply relative my-2 rounded-sm overflow-hidden;
  border: 1px solid var(--theme-border);
  background-color: var(--theme-surface-bg);
}

.chat-body-md :deep(.md-codeblock-lang) {
  @apply pointer-events-none absolute right-9 top-1 text-[0.68rem] uppercase;
  color: var(--theme-fg-faint);
  font-family: var(--theme-font-mono);
  letter-spacing: 0.5px;
}

.chat-body-md :deep(.md-codeblock pre) {
  @apply m-0 overflow-x-auto px-3 py-2 text-[0.82rem] leading-snug;
  font-family: var(--theme-font-mono);
  background: transparent !important;
}

.chat-body-md :deep(.md-codeblock pre code) {
  @apply bg-transparent p-0 text-inherit;
}

.chat-body-md :deep(.md-codeblock .md-copy) {
  @apply absolute right-1 top-1 cursor-pointer rounded-sm border px-2 py-[1px] text-[0.68rem] transition-colors;
  color: var(--theme-fg-dim);
  background-color: var(--theme-surface);
  border-color: var(--theme-border);
  font-family: var(--theme-font-mono);
}

.chat-body-md :deep(.md-codeblock .md-copy:hover) {
  color: var(--theme-fg);
  border-color: var(--theme-border-focus);
}

.chat-body-md :deep(.md-codeblock .md-copy[data-copied='true']) {
  color: var(--theme-status-ok);
  border-color: var(--theme-status-ok);
}

.chat-body-md :deep(table) {
  @apply my-1 w-full border-collapse;
}

.chat-body-md :deep(th),
.chat-body-md :deep(td) {
  @apply px-2 py-1 text-left;
  border: 1px solid var(--theme-border);
}

.chat-body-md :deep(th) {
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg-ink-2);
}

.chat-body-md :deep(.task-list-item) {
  @apply list-none;
}

.chat-body-md :deep(.task-list-item-checkbox) {
  @apply mr-1;
}
</style>
