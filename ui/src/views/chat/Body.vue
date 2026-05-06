<script setup lang="ts">
import { computed, useSlots } from 'vue'

import { MarkdownBody, Role } from '@components'

/**
 * Body card for both roles. Default behaviour preserves the slot —
 * both roles render the slot's text content into a styled lane, so
 * `<ChatBody :role="Role.User">{{ text }}</ChatBody>` keeps working.
 *
 * Setting `:markdown` + `:text` switches the lane to `<MarkdownBody>`
 * (markdown-it + Shiki + DOMPurify, with code-block chrome — copy +
 * collapse — wired in the component itself). Both user- and
 * assistant-role text route through markdown when the prop is set —
 * captains type pasted code blocks / lists / headings into the
 * composer too, and rendering them as plain text reads as second-
 * class compared to the agent's reply.
 */
const props = defineProps<{
  role: Role
  /** Optional markdown source. Required when `markdown` is true. */
  text?: string
  /** Render `text` through the markdown pipeline. */
  markdown?: boolean
}>()

const useMarkdown = computed(() => props.markdown === true && typeof props.text === 'string')
const slots = useSlots()
const slotEmpty = computed(() => !slots.default)
</script>

<template>
  <div class="chat-body" :data-role="role">
    <MarkdownBody v-if="useMarkdown && text" :source="text" />
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
  /* Round the corners that aren't connected to the role stripe so
   * the body reads as a card flush against the lane stripe. The
   * left edge stays squared because the stripe is the body's
   * left frame. 4px matches every other container surface
   * (composer, modal, palette card). */
  border-top-right-radius: 4px;
  border-bottom-right-radius: 4px;
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

/* Scoped overrides on top of MarkdownBody — chat surfaces use a
 * smaller body font + tighter line-height than MarkdownBody's spec-
 * sheet defaults. Code-block chrome (.md-codeblock-*) lives inside
 * MarkdownBody and stays untouched. */
.chat-body :deep(.markdown-body) {
  font-size: inherit;
  line-height: inherit;
}

.chat-body :deep(.markdown-body p) {
  @apply my-1;
}

.chat-body :deep(.markdown-body ul),
.chat-body :deep(.markdown-body ol) {
  @apply my-1 pl-5;
  font-size: inherit;
  line-height: inherit;
}

.chat-body :deep(.markdown-body li) {
  @apply my-0.5;
  font-size: inherit;
  line-height: inherit;
}

/* Headings inside chat prose: prose is a stream of paragraphs +
 * lists, so headings shouldn't grow far past body text. Cap at the
 * body size + a slim weight bump. */
.chat-body :deep(.markdown-body h1),
.chat-body :deep(.markdown-body h2),
.chat-body :deep(.markdown-body h3),
.chat-body :deep(.markdown-body h4),
.chat-body :deep(.markdown-body h5),
.chat-body :deep(.markdown-body h6) {
  @apply my-2 font-semibold;
  font-size: inherit;
  line-height: 1.3;
  color: var(--theme-fg);
}

.chat-body :deep(.markdown-body h1) {
  font-size: 1.05em;
}
.chat-body :deep(.markdown-body h2) {
  font-size: 1em;
}
</style>
