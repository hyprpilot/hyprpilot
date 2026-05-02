<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

/**
 * Modal body primitive — single-line text input with autoselect on
 * mount, `@submit` on Enter, and an inline-error pill when
 * `validate` returns a non-empty string. Composable inside
 * `<Modal>`'s default slot, typically paired with `<ModalDescription>`.
 *
 * Per CLAUDE.md naming rule the v-model props are bare:
 * `v-model:value` — the modal-scope context already says "input".
 *
 * `validate` is optional. Returning a string surfaces it as a
 * dim-red error pill below the input and disables the implicit
 * `submit` event (the parent's "save" button can still ignore the
 * gate, but the keyboard-Enter path bails so a single Enter
 * keystroke doesn't ship invalid input).
 */
const props = withDefaults(
  defineProps<{
    /** v-model:value — current input string. */
    value: string
    /** Placeholder when empty. */
    placeholder?: string
    /**
     * Optional validator. Return `null` for valid, or a one-line
     * error message that renders as a dim-red pill below the input.
     * Stateless — caller-owned; the input only displays the result.
     */
    validate?: (_raw: string) => string | null
  }>(),
  { placeholder: '', validate: undefined }
)

const emit = defineEmits<{
  'update:value': [next: string]
  submit: []
}>()

const inputRef = ref<HTMLInputElement>()

onMounted(() => {
  // Autoselect so the captain can immediately type-over the
  // pre-filled value without reaching for the mouse / Ctrl+A.
  inputRef.value?.select()
})

function onInput(e: Event): void {
  const target = e.target as HTMLInputElement

  emit('update:value', target.value)
}

function onKeydown(e: KeyboardEvent): void {
  if (e.key !== 'Enter') {
    return
  }
  // Suppress submit while validation flags an error — the captain
  // sees the pill, fixes, then commits. Parent button-click path is
  // unaffected; this gate is only for the implicit Enter shortcut.
  const err = props.validate ? props.validate(props.value) : null

  if (err) {
    return
  }
  e.preventDefault()
  emit('submit')
}

const errorMessage = computed<string | null>(() => (props.validate ? props.validate(props.value) : null))
</script>

<template>
  <div class="modal-input-wrap">
    <input ref="inputRef" type="text" class="modal-input" :value="value" :placeholder="placeholder" :data-invalid="errorMessage !== null" @input="onInput" @keydown="onKeydown" />
    <span v-if="errorMessage" class="modal-input-error">{{ errorMessage }}</span>
  </div>
</template>

<style scoped>
@reference '../assets/styles.css';

.modal-input-wrap {
  @apply flex flex-col gap-1;
}

.modal-input {
  @apply w-full;
  padding: 6px 8px;
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
  border: 1px solid var(--theme-border);
  border-radius: 3px;
  font-family: var(--theme-font-mono);
  font-size: 0.78rem;
  outline: none;
}

.modal-input:focus {
  border-color: var(--theme-border-focus);
}

.modal-input[data-invalid='true'] {
  border-color: var(--theme-status-err);
}

.modal-input-error {
  color: var(--theme-status-err);
  font-family: var(--theme-font-mono);
  font-size: 0.65rem;
}
</style>
