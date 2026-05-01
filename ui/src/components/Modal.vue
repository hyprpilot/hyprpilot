<script setup lang="ts">
import type { IconDefinition } from '@fortawesome/fontawesome-svg-core'
import { computed } from 'vue'

import { ToastTone } from '@components'

/**
 * Generic centered modal: backdrop + card + tagged header (icon + label
 * + tone-coloured tag bg) + an `#actions` slot for the right-side
 * button row + a default slot for the body. The card sits
 * absolutely-positioned over the closest `position: relative`
 * ancestor — drop it next to a `chat-transcript` etc. and it scopes
 * inside that container instead of the viewport.
 *
 * Body rendering is the caller's choice — pass `<MarkdownBody />`,
 * `<TextBody />`, or any other SFC. Per CLAUDE.md's compose-not-bag
 * rule, Modal does not pattern-match over a `markdown` / `text` prop
 * bag; the renderer IS the slot content.
 *
 * Action buttons land in the `#actions` slot — never as a structured
 * `actions: ModalAction[]` prop. The consumer reaches for `<Button>` /
 * a custom SFC / inline elements as the situation calls for.
 */

const props = withDefaults(
  defineProps<{
    /// Tag label rendered inside the tone-bg pill in the header.
    title: string
    /// Tone driving the header pill bg + the modal's top border colour.
    tone?: ToastTone
    /// FontAwesome icon for the header tag. Pass the imported
    /// `IconDefinition` directly (`faListCheck`), never the legacy
    /// `['fas', 'list-check']` tuple — direct imports tree-shake.
    icon?: IconDefinition
    /// When `true`, clicking the backdrop emits `dismiss`.
    dismissable?: boolean
  }>(),
  {
    tone: ToastTone.Warn,
    dismissable: true
  }
)

const emit = defineEmits<{
  dismiss: []
}>()

function onBackdropClick(): void {
  if (props.dismissable) {
    emit('dismiss')
  }
}

function toneBg(tone: ToastTone): string {
  switch (tone) {
    case ToastTone.Ok:
      return 'var(--theme-status-ok)'
    case ToastTone.Err:
      return 'var(--theme-status-err)'
    case ToastTone.Warn:
    default:
      return 'var(--theme-status-warn)'
  }
}

const headerBg = computed(() => toneBg(props.tone))
</script>

<template>
  <div class="modal-backdrop" role="dialog" aria-modal="true" :aria-label="title" @click.self="onBackdropClick">
    <article class="modal" :data-tone="tone">
      <header class="modal-header">
        <span class="modal-tag" :style="{ backgroundColor: headerBg }">
          <FaIcon v-if="icon" :icon="icon" class="modal-tag-icon" aria-hidden="true" />
          <span class="modal-tag-label">{{ title }}</span>
        </span>
        <span class="modal-spacer" />
        <div class="modal-actions">
          <slot name="actions" />
        </div>
      </header>

      <div class="modal-body">
        <slot />
      </div>
    </article>
  </div>
</template>

<style scoped>
@reference '../assets/styles.css';

.modal-backdrop {
  position: absolute;
  inset: 0;
  z-index: 30;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background-color: rgba(var(--theme-surface-bg-rgb), 0.85);
}

.modal {
  @apply flex flex-col;
  width: 100%;
  max-width: 720px;
  max-height: 85%;
  background-color: var(--theme-surface);
  border: 2px solid var(--theme-status-warn);
  border-radius: 4px;
  overflow: hidden;
  box-shadow: 0 8px 24px rgb(0 0 0 / 0.4);
}

.modal[data-tone='ok'] {
  border-color: var(--theme-status-ok);
}

.modal[data-tone='err'] {
  border-color: var(--theme-status-err);
}

.modal-header {
  @apply flex items-center;
  gap: 8px;
  padding: 8px 12px;
  background-color: var(--theme-permission-bg);
  border-bottom: 1px solid var(--theme-border-soft);
  font-family: var(--theme-font-mono);
}

.modal-tag {
  @apply inline-flex items-center;
  gap: 6px;
  padding: 3px 9px;
  color: var(--theme-fg-on-tone);
  border-radius: 3px;
  font-size: 0.65rem;
  font-weight: 700;
  letter-spacing: 0.3px;
}

.modal-tag-icon {
  width: 9px;
  height: 9px;
}

.modal-tag-label {
  font-weight: 700;
}

.modal-spacer {
  flex: 1;
}

.modal-actions {
  @apply inline-flex items-center;
  gap: 6px;
}

.modal-body {
  flex: 1 1 auto;
  overflow-y: auto;
  padding: 14px 18px;
  background-color: var(--theme-surface-bg);
}
</style>
