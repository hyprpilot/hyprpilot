<script setup lang="ts">
import { nextTick, onMounted, ref, watch } from 'vue'

import type { ComposerPill } from '../types'
import { type KeymapEntry, useKeymap, useKeymaps } from '@composables'
import { log } from '@lib'

/**
 * Composer row: pills (attachments / resources) + autosizing textarea +
 * send button. Port of D5's `D5Composer`. No submit wiring — the parent
 * decides what `submit` means.
 *
 * design-skip: `reference/*` pill variant dropped — reviewer decision:
 * "no references". Bundle carries it, port omits it; any caller passing
 * a pill with `kind='reference'` just renders the fallback style.
 */
const props = withDefaults(
  defineProps<{
    pills?: ComposerPill[]
    placeholder?: string
    disabled?: boolean
    sending?: boolean
  }>(),
  {
    pills: () => [],
    placeholder: 'message pilot',
    disabled: false,
    sending: false
  }
)

const emit = defineEmits<{
  submit: [text: string]
  removePill: [id: string]
}>()

const text = ref('')
const textareaRef = ref<HTMLTextAreaElement>()

// Autosize the textarea: grow from ~5 lines minimum up to 25vh cap, then
// scroll internally. Runs after every text change + on mount so the
// initial render already has the 5-line footprint.
function resize(): void {
  const el = textareaRef.value
  if (!el) {
    return
  }
  el.style.height = 'auto'
  el.style.height = `${el.scrollHeight}px`
}

const { keymaps } = useKeymaps()
// Listener scopes to the textarea — fires only while it owns focus.
useKeymap(textareaRef, (): KeymapEntry[] => {
  if (!keymaps.value) {
    return []
  }

  return [
    { binding: keymaps.value.chat.submit, handler: onEnter },
    // Explicit no-op so Shift+Enter falls through to the textarea's
    // native newline insertion (and doesn't match chat.submit).
    { binding: keymaps.value.chat.newline, handler: () => false },
    { binding: keymaps.value.composer.paste_image, handler: onPasteImage },
    { binding: keymaps.value.composer.tab_completion, handler: onTab },
    { binding: keymaps.value.composer.shift_tab, handler: onTab },
    { binding: keymaps.value.composer.history_up, handler: onHistoryPrev, allowRepeat: true },
    { binding: keymaps.value.composer.history_down, handler: onHistoryNext, allowRepeat: true }
  ]
})

onMounted(() => {
  resize()
})

watch(text, () => nextTick(resize))

defineExpose({
  clear(): void {
    text.value = ''
    nextTick(resize)
  }
})

function trySubmit(): void {
  const val = text.value.trim()
  if (!val || props.sending || props.disabled) {
    return
  }
  emit('submit', val)
}

function onEnter(e: KeyboardEvent): boolean {
  if (e.isComposing) {
    return false
  }
  log.debug('composer keybind', { key: 'Enter' })
  trySubmit()

  return true
}

function onTab(): boolean {
  log.debug('composer keybind', { key: 'Tab', target: 'completion' })

  return false
}

function onPasteImage(): boolean {
  // TODO(K-image): wire the clipboard-image handler when image attachments land.
  log.debug('composer keybind', { key: 'ctrl+p', target: 'paste-image' })

  return false
}

function onHistoryPrev(): boolean {
  // TODO(K-history): composer history store not yet wired.
  log.debug('composer keybind', { key: 'ctrl+arrowup', target: 'history-prev' })

  return false
}

function onHistoryNext(): boolean {
  // TODO(K-history): composer history store not yet wired.
  log.debug('composer keybind', { key: 'ctrl+arrowdown', target: 'history-next' })

  return false
}
</script>

<template>
  <form class="composer" data-testid="composer" @submit.prevent="trySubmit">
    <div v-if="pills.length > 0" class="composer-pills">
      <span v-for="p in pills" :key="p.id" class="composer-pill" :data-kind="p.kind">
        <span class="composer-pill-label">{{ p.label }}</span>
        <button type="button" class="composer-pill-remove" aria-label="remove" @click="emit('removePill', p.id)">
          <FaIcon :icon="['fas', 'xmark']" class="composer-pill-remove-icon" />
        </button>
      </span>
    </div>

    <div class="composer-row">
      <textarea ref="textareaRef" v-model="text" class="composer-textarea" rows="5" :placeholder="placeholder" :disabled="disabled" data-testid="composer-textarea" />
      <button
        type="submit"
        class="composer-submit"
        :aria-label="sending ? 'sending' : 'send'"
        :disabled="sending || disabled || text.trim().length === 0"
        data-testid="composer-submit"
      >
        <FaIcon :icon="['fas', 'reply']" class="composer-submit-icon" aria-hidden="true" />
      </button>
    </div>
  </form>
</template>

<style scoped>
@reference '../../assets/styles.css';

.composer {
  @apply flex flex-col gap-1 px-3 py-2;
  background-color: var(--theme-surface);
}

.composer-pills {
  @apply flex flex-wrap items-center gap-1;
}

.composer-pill {
  @apply inline-flex items-center gap-1 border px-2 py-[2px] text-[0.7rem] leading-tight;
  font-family: var(--theme-font-mono);
  color: var(--theme-fg-ink-2);
  background-color: var(--theme-surface-alt);
  border-color: var(--theme-border);
}

.composer-pill-label {
  @apply truncate;
  max-width: 18ch;
}

.composer-pill-remove {
  @apply border-0 bg-transparent px-0 text-[0.7rem] leading-none;
  color: var(--theme-fg-dim);
  cursor: pointer;
}

.composer-pill-remove-icon {
  width: 9px;
  height: 9px;
}

.composer-pill-remove:hover {
  color: var(--theme-status-err);
}

.composer-row {
  @apply flex items-end gap-2;
  min-width: 0;
}

.composer-textarea {
  @apply w-full min-w-0 flex-1 resize-none overflow-y-auto border px-2 py-1 text-[0.85rem] leading-snug;
  font-family: var(--theme-font-mono);
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg);
  border-color: var(--theme-border);
  /* rows="5" sets the initial visible lines; max-height caps autosize at 25vh. */
  max-height: 25vh;
}

.composer-textarea:focus {
  outline: none;
  border-color: var(--theme-accent);
}

.composer-textarea:disabled {
  opacity: 0.5;
}

.composer-submit {
  @apply shrink-0 self-stretch border-0 px-[14px] py-2 font-bold text-[0.82rem];
  font-family: var(--theme-font-mono);
  color: var(--theme-surface-bg);
  background-color: var(--theme-accent-assistant);
  cursor: pointer;
}

.composer-submit-icon {
  width: 13px;
  height: 13px;
}

.composer-submit:hover:not(:disabled) {
  background-color: color-mix(in srgb, var(--theme-accent-assistant) 85%, black);
}

.composer-submit:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>
