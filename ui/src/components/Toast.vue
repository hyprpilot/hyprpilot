<script setup lang="ts">
import { faXmark } from '@fortawesome/free-solid-svg-icons'
import { computed, h, type VNode } from 'vue'

import { ToastTone } from '@components'
import type { ToastBody } from '@composables'

/**
 * In-Frame toast card — absolute-positioned over the chat body
 * (top: 8, left/right: 14, z-index: 10), 3px tone-stripe left
 * border, body content in fg-ink-2, trailing line, ✕ dismiss.
 *
 * The `body` accepts a string (rendered in the standard message
 * span), a render function (`() => VNode` — the consumer composes
 * label + button + whatever inline), or `{ component, props }`
 * (lifts a small SFC for richer toasts). No more `actionLabel` /
 * `@action` — the consumer wires whatever interaction it needs
 * inside the body itself.
 *
 * Tone is encoded **only** through the left-stripe color — no
 * "NOTICE" / "ERROR" tag word. The color carries the level signal.
 */
const props = defineProps<{
  tone: ToastTone
  body: ToastBody
}>()

defineEmits<{
  dismiss: []
}>()

const toneColor = computed(() => {
  switch (props.tone) {
    case ToastTone.Err:
      return 'var(--theme-status-err)'
    case ToastTone.Ok:
      return 'var(--theme-status-ok)'
    default:
      return 'var(--theme-status-warn)'
  }
})

/**
 * Inline functional render for the body slot. Vue 3 picks up
 * arrow-functions defined in `<script setup>` as functional
 * components when used in template (`<RenderBody />`). Branches
 * on `body`'s discriminator: string → text span, function → call
 * it, `{ component, props }` → mount that component.
 */
function RenderBody(): VNode | string | null {
  const body = props.body
  if (typeof body === 'string') {
    return h('span', { class: 'toast-message' }, body)
  }
  if (typeof body === 'function') {
    return body()
  }
  if (body && typeof body === 'object' && 'component' in body) {
    return h(body.component, body.props ?? {})
  }
  return null
}
</script>

<template>
  <div class="toast" :style="{ '--tone': toneColor }" data-testid="toast">
    <RenderBody />
    <span class="toast-line" />
    <button type="button" class="toast-dismiss" aria-label="dismiss" @click="$emit('dismiss')">
      <FaIcon :icon="faXmark" class="toast-dismiss-icon" aria-hidden="true" />
    </button>
  </div>
</template>

<style scoped>
@reference '../assets/styles.css';

/* Wireframe spec — overlay card pinned to the top of the Frame body
 * via absolute positioning (the parent `.frame-body` is relative). */
.toast {
  position: absolute;
  top: 8px;
  left: 14px;
  right: 14px;
  z-index: 10;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px 6px 10px;
  background-color: var(--theme-surface);
  border: 1px solid var(--theme-border-soft);
  border-left: 3px solid var(--tone);
  border-radius: 3px;
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.45);
  font-family: var(--theme-font-mono);
  font-size: 0.56rem;
  color: var(--theme-fg-dim);
  text-transform: uppercase;
  letter-spacing: 1px;
}

.toast :deep(.toast-message) {
  color: var(--theme-fg-ink-2);
  font-size: 0.66rem;
  text-transform: none;
  letter-spacing: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.toast-line {
  flex: 1;
  height: 1px;
  background-color: var(--theme-border);
  margin-left: 4px;
}

.toast-dismiss {
  @apply inline-flex items-center justify-center;
  color: var(--theme-fg-dim);
  cursor: pointer;
  padding: 0 4px;
  background: transparent;
  border: 0;
}

.toast-dismiss-icon {
  width: 9px;
  height: 9px;
}

.toast-dismiss:hover {
  color: var(--theme-fg);
}
</style>
