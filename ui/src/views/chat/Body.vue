<script setup lang="ts">
import { writeText as writeClipboardText } from '@tauri-apps/plugin-clipboard-manager'
import { computed, ref, useSlots, watch } from 'vue'

import { Role } from '@components'
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

const renderedHtml = ref('')
const useMarkdown = computed(() => props.markdown === true && props.role === Role.Assistant)
const slots = useSlots()
const slotEmpty = computed(() => !slots.default)

watch(
  [() => props.text, useMarkdown],
  async([raw, on]) => {
    if (!on || !raw) {
      renderedHtml.value = ''

      return
    }

    try {
      const out = await renderMarkdown(raw)

      renderedHtml.value = out.html
    } catch(err) {
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
  // Stop the click bubbling up to `[data-md-toggle]` (the same `<header>`
  // the copy button lives inside) — copying shouldn't also collapse.
  event.stopPropagation()
  const block = button.closest('.md-codeblock')
  const code = block?.querySelector('pre code')?.textContent ?? ''

  if (!code) {
    return
  }
  // Tauri clipboard plugin (arboard under the hood) instead of
  // `navigator.clipboard.writeText` — on Wayland + WebKitGTK the web
  // Clipboard API can land on the wrong selection (PRIMARY vs
  // CLIPBOARD), and a layer-shell surface without focus may have no
  // permission to write at all. The plugin writes to the OS-level
  // CLIPBOARD via the compositor's wlr-data-control protocol, which
  // works regardless of webview focus / surface role.
  void writeClipboardText(code)
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

/**
 * Code-block collapse toggle. Header carries `data-md-toggle`; click
 * (or Enter / Space) flips `data-collapsed` on the parent
 * `.md-codeblock` which the scoped CSS uses to hide the body and
 * swap the caret. Default on render is `data-collapsed="false"` —
 * the user said "not collapsed by default".
 */
function onMdRootClick(event: MouseEvent): void {
  onCopyClick(event)
  const target = event.target as HTMLElement | null
  const header = target?.closest('[data-md-toggle]') as HTMLElement | null

  if (!header) {
    return
  }

  // Skip the toggle when the click landed inside the copy button —
  // `onCopyClick` already returned and we don't want a copy click to
  // also collapse the block.
  if (target?.closest('button[data-md-copy]')) {
    return
  }
  const block = header.closest('.md-codeblock') as HTMLElement | null

  if (!block) {
    return
  }
  const next = block.dataset.collapsed === 'true' ? 'false' : 'true'

  block.dataset.collapsed = next
}

function onMdRootKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Enter' && event.key !== ' ') {
    return
  }
  const target = event.target as HTMLElement | null
  const header = target?.closest('[data-md-toggle]') as HTMLElement | null

  if (!header) {
    return
  }
  event.preventDefault()
  const block = header.closest('.md-codeblock') as HTMLElement | null

  if (!block) {
    return
  }
  const next = block.dataset.collapsed === 'true' ? 'false' : 'true'

  block.dataset.collapsed = next
}
</script>

<template>
  <div class="chat-body" :data-role="role">
    <!-- eslint-disable-next-line vue/no-v-html -- HTML is sanitised by renderMarkdown -->
    <div v-if="useMarkdown && renderedHtml" class="chat-body-md prose" v-html="renderedHtml" @click="onMdRootClick" @keydown="onMdRootKeydown" />
    <div v-else-if="useMarkdown && !renderedHtml && text" class="chat-body-plain">{{ text }}</div>
    <slot v-else-if="!slotEmpty" />
    <div v-else-if="text" class="chat-body-plain">{{ text }}</div>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* Body card sits inside the turn lane and shares the lane's left
 * stripe — so no own left border. The turn parent (`Turn.vue`) owns
 * the role-color stripe; the body frames its top / right / bottom
 * edges and reads as one continuous lane.
 *
 * Role tint is layered: solid `--theme-surface-bg` underneath, a
 * `::before` pseudo at `inset: 0` painted with `rgba(<accent>, .14)`
 * over the top. We use RGBA (CSS3, broadly supported) instead of
 * `color-mix(...)` because the WebKit2GTK 4.1 runtime predates the
 * `color-mix` spec — any `color-mix` declaration silently no-ops
 * there, leaving the body identical to the surface. The triplet
 * (`--theme-accent-X-rgb`) is emitted by `applyTheme` for every
 * hex theme leaf, so changing `accent.user` retints both the lane
 * stripe and the body fill in lockstep. */
.chat-body {
  @apply px-3 py-2 text-[0.78rem] leading-snug relative isolate;
  color: var(--theme-fg);
  background-color: var(--theme-surface);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  overflow-wrap: anywhere;
  min-width: 0;
  font-family: var(--theme-font-sans);
}

.chat-body::before {
  content: '';
  position: absolute;
  inset: 0;
  pointer-events: none;
  z-index: -1;
}

.chat-body[data-role='assistant']::before {
  background-color: rgba(var(--theme-accent-assistant-rgb), 0.01);
}

.chat-body[data-role='user'] {
  white-space: pre-wrap;
}

.chat-body[data-role='user']::before {
  background-color: rgba(var(--theme-accent-user-rgb), 0.01);
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
  font-size: inherit;
  line-height: inherit;
}

.chat-body-md :deep(ul) {
  list-style-type: disc;
}

.chat-body-md :deep(ol) {
  list-style-type: decimal;
}

.chat-body-md :deep(li) {
  @apply my-0.5;
  font-size: inherit;
  line-height: inherit;
}

/* Headings inside chat prose: prose is a stream of paragraphs +
 * lists, so headings shouldn't grow far past body text. Cap at
 * the body size + a slim weight bump — what the wireframe spec
 * calls "section break", not "page banner". */
.chat-body-md :deep(h1),
.chat-body-md :deep(h2),
.chat-body-md :deep(h3),
.chat-body-md :deep(h4),
.chat-body-md :deep(h5),
.chat-body-md :deep(h6) {
  @apply my-2 font-semibold;
  font-size: inherit;
  line-height: 1.3;
  color: var(--theme-fg);
}

.chat-body-md :deep(h1) {
  font-size: 1.05em;
}
.chat-body-md :deep(h2) {
  font-size: 1em;
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
  @apply my-2 rounded-sm overflow-hidden;
  border: 1px solid var(--theme-border);
  background-color: var(--theme-surface-bg);
}

/* Header bar — chevron + language + spacer + copy. Flex layout
 * (replaces the old absolute-positioned lang/copy that overlapped
 * each other on narrow code blocks). Click anywhere on the header
 * (except the copy button) toggles `data-collapsed` on the parent. */
.chat-body-md :deep(.md-codeblock-header) {
  @apply flex items-center gap-2 cursor-pointer;
  padding: 4px 6px 4px 8px;
  border-bottom: 1px solid var(--theme-border);
  background-color: var(--theme-surface);
  font-family: var(--theme-font-mono);
  user-select: none;
}

.chat-body-md :deep(.md-codeblock[data-collapsed='true'] .md-codeblock-header) {
  border-bottom: 0;
}

.chat-body-md :deep(.md-codeblock-icon) {
  width: 9px;
  height: 9px;
  display: inline-block;
  flex-shrink: 0;
}

.chat-body-md :deep(.md-codeblock-caret) {
  @apply inline-flex items-center justify-center;
  color: var(--theme-fg-dim);
  width: 10px;
}

/* Caret swap by collapse state. The HTML emits both icons; CSS
 * shows only the matching one. */
.chat-body-md :deep(.md-codeblock[data-collapsed='false'] [data-md-caret-right]) {
  display: none;
}
.chat-body-md :deep(.md-codeblock[data-collapsed='true'] [data-md-caret-down]) {
  display: none;
}
.chat-body-md :deep(.md-codeblock[data-collapsed='true'] [data-md-caret-right]) {
  display: inline-flex;
}

.chat-body-md :deep(.md-codeblock-lang) {
  @apply text-[0.62rem] uppercase;
  color: var(--theme-fg-faint);
  letter-spacing: 0.6px;
}

.chat-body-md :deep(.md-codeblock-spacer) {
  flex: 1;
}

.chat-body-md :deep(.md-codeblock-body) {
  display: block;
}

.chat-body-md :deep(.md-codeblock[data-collapsed='true'] .md-codeblock-body) {
  display: none;
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
  @apply inline-flex items-center gap-1 cursor-pointer rounded-sm border px-[6px] py-[2px] text-[0.6rem] transition-colors;
  color: var(--theme-fg-dim);
  background-color: var(--theme-surface-bg);
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
