<script setup lang="ts">
import { writeText as writeClipboardText } from '@tauri-apps/plugin-clipboard-manager'
import { ref, watch } from 'vue'

import { log, renderMarkdown } from '@lib'

/**
 * Renders a markdown source through the shared Shiki + DOMPurify
 * pipeline, AND owns the fenced-code-block chrome (collapse caret +
 * copy button). Every consumer (Body.vue transcript bodies,
 * ToolBody.vue tool descriptions, Modal bodies, future surfaces)
 * gets the working chrome by default — there is no opt-in. Consumers
 * layer their own prose styling on top via `:deep(.markdown-body ...)`
 * scoped rules.
 *
 * Click + keyboard delegation lives here too — `renderMarkdown` emits
 * `data-md-toggle` / `data-md-copy` hooks; we wire the handlers on
 * the v-html root so the chrome works no matter where the body lands.
 */
const props = defineProps<{ source: string }>()

const html = ref('')

watch(
  () => props.source,
  async(raw) => {
    if (!raw) {
      html.value = ''

      return
    }

    try {
      const out = await renderMarkdown(raw)

      html.value = out.html
    } catch(err) {
      log.warn('MarkdownBody: render failed', { err: String(err) })
      html.value = ''
    }
  },
  { immediate: true }
)

/**
 * Copy the fenced block's code to the OS clipboard. Tauri clipboard
 * plugin (arboard under the hood) instead of `navigator.clipboard.writeText`
 * — on Wayland + WebKitGTK the web Clipboard API can land on the wrong
 * selection (PRIMARY vs CLIPBOARD), and a layer-shell surface without
 * focus may have no permission to write at all. The plugin writes to
 * the OS-level CLIPBOARD via the compositor's wlr-data-control
 * protocol, regardless of webview focus / surface role.
 */
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
 * swap the caret. Default on render is `data-collapsed="false"`.
 */
function onRootClick(event: MouseEvent): void {
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
  block.dataset.collapsed = block.dataset.collapsed === 'true' ? 'false' : 'true'
}

function onRootKeydown(event: KeyboardEvent): void {
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
  block.dataset.collapsed = block.dataset.collapsed === 'true' ? 'false' : 'true'
}
</script>

<template>
  <!-- eslint-disable-next-line vue/no-v-html -- HTML is sanitised by renderMarkdown -->
  <div v-if="html" class="markdown-body" v-html="html" @click="onRootClick" @keydown="onRootKeydown" />
  <pre v-else class="markdown-fallback">{{ source }}</pre>
</template>

<style scoped>
@reference '../assets/styles.css';

.markdown-body {
  font-family: var(--theme-font-sans);
  font-size: 0.85rem;
  line-height: 1.5;
  color: var(--theme-fg);
}

.markdown-body :deep(p) {
  margin: 6px 0;
}

.markdown-body :deep(p:first-child) {
  margin-top: 0;
}

.markdown-body :deep(p:last-child) {
  margin-bottom: 0;
}

.markdown-body :deep(ul),
.markdown-body :deep(ol) {
  margin: 6px 0;
  padding-left: 22px;
  font-size: inherit;
  line-height: inherit;
}

.markdown-body :deep(ul) {
  list-style-type: disc;
}

.markdown-body :deep(ol) {
  list-style-type: decimal;
}

.markdown-body :deep(li) {
  margin: 2px 0;
}

/* GFM task-list checkboxes (markdown-it-task-lists) — drop the disc
 * bullet so the row reads as `[ ] text` instead of `• [ ]    text`.
 * `:has()` would target the parent `<ul>` to also strip its
 * padding-left, but webkit2gtk 4.1 (Tauri's Linux runtime) predates
 * `:has`; we live with the inherited `<ul>` indent — `[ ]` aligned
 * one indent in reads fine. */
.markdown-body :deep(li.task-list-item) {
  list-style: none;
}

.markdown-body :deep(.task-list-item-checkbox) {
  margin: 0 6px 0 0;
  vertical-align: middle;
}

.markdown-body :deep(h1),
.markdown-body :deep(h2),
.markdown-body :deep(h3),
.markdown-body :deep(h4),
.markdown-body :deep(h5),
.markdown-body :deep(h6) {
  margin: 12px 0 6px;
  font-weight: 700;
  color: var(--theme-fg);
  line-height: 1.3;
}

.markdown-body :deep(h1) {
  font-size: 1.15em;
}
.markdown-body :deep(h2) {
  font-size: 1.05em;
}
.markdown-body :deep(h3) {
  font-size: 1em;
}

.markdown-body :deep(blockquote) {
  margin: 6px 0;
  padding-left: 8px;
  border-left: 2px solid var(--theme-border-soft);
  color: var(--theme-fg-dim);
}

.markdown-body :deep(a) {
  color: var(--theme-accent);
  text-decoration: underline;
  text-underline-offset: 2px;
}

/* Inline code only — fenced blocks below override via .md-codeblock
 * descendant selectors. */
.markdown-body :deep(code) {
  font-family: var(--theme-font-mono);
  font-size: 0.85em;
  padding: 1px 4px;
  border-radius: 3px;
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
}

/* Fallback prose <pre> when a fence couldn't be highlighted (unknown
 * language). The .md-codeblock chrome below replaces it for known
 * languages. */
.markdown-body :deep(pre) {
  margin: 8px 0;
  padding: 8px 10px;
  background-color: var(--theme-surface-alt);
  border: 1px solid var(--theme-border-soft);
  border-radius: 3px;
  overflow-x: auto;
}

.markdown-body :deep(pre code) {
  background-color: transparent;
  padding: 0;
}

/* Fenced-block chrome — caret + lang + spacer + copy. Shared across
 * every consumer; consumers DO NOT re-style this. */
.markdown-body :deep(.md-codeblock) {
  margin: 8px 0;
  border: 1px solid var(--theme-border);
  border-radius: 3px;
  background-color: var(--theme-surface-bg);
  overflow: hidden;
}

.markdown-body :deep(.md-codeblock-header) {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  padding: 4px 6px 4px 8px;
  border-bottom: 1px solid var(--theme-border);
  background-color: var(--theme-surface);
  font-family: var(--theme-font-mono);
  user-select: none;
}

.markdown-body :deep(.md-codeblock[data-collapsed='true'] .md-codeblock-header) {
  border-bottom: 0;
}

.markdown-body :deep(.md-codeblock-caret) {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--theme-fg-dim);
  width: 10px;
}

/* Caret swap by collapse state. The HTML emits both icons; CSS
 * shows only the matching one. */
.markdown-body :deep(.md-codeblock[data-collapsed='false'] [data-md-caret-right]) {
  display: none;
}
.markdown-body :deep(.md-codeblock[data-collapsed='true'] [data-md-caret-down]) {
  display: none;
}
.markdown-body :deep(.md-codeblock[data-collapsed='true'] [data-md-caret-right]) {
  display: inline-flex;
}

.markdown-body :deep(.md-codeblock-lang) {
  font-size: 0.62rem;
  text-transform: uppercase;
  color: var(--theme-fg-faint);
  letter-spacing: 0.6px;
}

.markdown-body :deep(.md-codeblock-spacer) {
  flex: 1;
}

.markdown-body :deep(.md-codeblock-body) {
  display: block;
}

.markdown-body :deep(.md-codeblock[data-collapsed='true'] .md-codeblock-body) {
  display: none;
}

.markdown-body :deep(.md-codeblock pre) {
  margin: 0;
  overflow-x: auto;
  padding: 8px 12px;
  font-size: 0.82rem;
  line-height: 1.45;
  font-family: var(--theme-font-mono);
  background: transparent !important;
  border: 0;
  border-radius: 0;
}

.markdown-body :deep(.md-codeblock pre code) {
  background-color: transparent;
  padding: 0;
  color: inherit;
}

/* Shiki diff-transformer line styling. `transformerNotationDiff`
 * tags `[!code ++]` / `[!code --]` annotated lines with `.diff.add`
 * / `.diff.remove` classes; we paint a soft tinted background +
 * a `+` / `-` gutter so the captain sees the diff at a glance.
 * Runs on top of full per-language Shiki highlighting — keeps
 * keyword colors, just frames the line. */
.markdown-body :deep(.md-codeblock pre .line.diff.add) {
  background-color: rgba(var(--theme-status-ok-rgb), 0.18);
  display: inline-block;
  width: 100%;
  position: relative;
  padding-left: 18px;
  margin-left: -8px;
}

.markdown-body :deep(.md-codeblock pre .line.diff.add)::before {
  content: '+';
  position: absolute;
  left: 4px;
  top: 0;
  color: var(--theme-status-ok);
  font-weight: 700;
}

.markdown-body :deep(.md-codeblock pre .line.diff.remove) {
  background-color: rgba(var(--theme-status-err-rgb), 0.18);
  display: inline-block;
  width: 100%;
  position: relative;
  padding-left: 18px;
  margin-left: -8px;
}

.markdown-body :deep(.md-codeblock pre .line.diff.remove)::before {
  content: '-';
  position: absolute;
  left: 4px;
  top: 0;
  color: var(--theme-status-err);
  font-weight: 700;
}

.markdown-body :deep(.md-codeblock .md-copy) {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  cursor: pointer;
  border-radius: 3px;
  border: 1px solid var(--theme-border);
  padding: 2px 6px;
  font-size: 0.6rem;
  font-family: var(--theme-font-mono);
  color: var(--theme-fg-dim);
  background-color: var(--theme-surface-bg);
  transition: color 0.12s, border-color 0.12s;
}

.markdown-body :deep(.md-codeblock .md-copy:hover) {
  color: var(--theme-fg);
  border-color: var(--theme-border-focus);
}

.markdown-body :deep(.md-codeblock .md-copy[data-copied='true']) {
  color: var(--theme-status-ok);
  border-color: var(--theme-status-ok);
}

.markdown-body :deep(table) {
  margin: 6px 0;
  width: 100%;
  border-collapse: collapse;
}

.markdown-body :deep(th),
.markdown-body :deep(td) {
  padding: 4px 8px;
  text-align: left;
  border: 1px solid var(--theme-border);
}

.markdown-body :deep(th) {
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg-subtle);
}

.markdown-fallback {
  margin: 0;
  overflow-x: auto;
  font-family: var(--theme-font-mono);
  font-size: 0.78rem;
  line-height: 1.4;
  white-space: pre-wrap;
  color: var(--theme-fg);
}
</style>
