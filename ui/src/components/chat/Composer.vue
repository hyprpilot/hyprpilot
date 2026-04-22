<script setup lang="ts">
import { useTextareaAutosize } from '@vueuse/core'
import { ref } from 'vue'

import { Button } from '@ui/button'

const props = defineProps<{
  disabled?: boolean
  sending?: boolean
  placeholder?: string
}>()

const emit = defineEmits<{
  submit: [text: string]
  cancel: []
}>()

const textareaEl = ref<HTMLTextAreaElement>()
const { input } = useTextareaAutosize({ element: textareaEl, input: '', styleProp: 'height' })

// Parent clears imperatively after `submit` resolves — a `sending`-edge watcher
// would also clear on failure, dropping the draft the user still wants to edit.
defineExpose({
  clear(): void {
    input.value = ''
  }
})

function trySubmit(): void {
  const text = input.value.trim()
  if (!text || props.sending || props.disabled) {
    return
  }
  emit('submit', text)
}

function onKeydown(e: KeyboardEvent): void {
  if (e.key !== 'Enter') {
    return
  }
  if (e.shiftKey) {
    return
  }
  if (e.isComposing) {
    return
  }
  e.preventDefault()
  trySubmit()
}
</script>

<template>
  <form class="composer" data-testid="composer" @submit.prevent="trySubmit">
    <textarea
      ref="textareaEl"
      v-model="input"
      class="composer-textarea"
      rows="1"
      :placeholder="placeholder ?? 'type a prompt — enter to send, shift-enter for newline'"
      :disabled="disabled"
      data-testid="composer-textarea"
      @keydown="onKeydown"
    />

    <div class="composer-actions">
      <Button type="submit" variant="accent" size="sm" :disabled="sending || disabled || input.trim().length === 0" data-testid="composer-submit">
        {{ sending ? 'sending…' : 'send' }}
      </Button>
      <Button type="button" variant="muted" size="sm" :disabled="!sending" data-testid="composer-cancel" @click="emit('cancel')"> cancel </Button>
    </div>
  </form>
</template>

<style scoped>
@reference "../../assets/styles.css";

.composer {
  @apply flex flex-col gap-2 border-t px-3 py-2;
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface-compose);
}

.composer-textarea {
  @apply w-full resize-none border px-2 py-1 text-[0.9rem] leading-snug;
  background-color: var(--theme-window);
  color: var(--theme-fg);
  border-color: var(--theme-border-soft);
  font-family: var(--theme-font-family);
  max-height: 12rem;

  &:focus {
    outline: none;
    border-color: var(--theme-border-focus);
  }

  &:disabled {
    opacity: 0.5;
  }
}

.composer-actions {
  @apply flex justify-end gap-2;
}
</style>
