<script setup lang="ts">
import { computed } from 'vue'

import { ButtonTone, ButtonVariant } from '@components'

const props = withDefaults(
  defineProps<{
    variant?: ButtonVariant
    tone?: ButtonTone
    disabled?: boolean
  }>(),
  {
    variant: ButtonVariant.Ghost,
    tone: ButtonTone.Neutral,
    disabled: false
  }
)

const emit = defineEmits<{
  click: [ev: MouseEvent]
}>()

const toneVar = computed(() => {
  switch (props.tone) {
    case ButtonTone.Ok:
      return 'var(--theme-status-ok)'
    case ButtonTone.Err:
      return 'var(--theme-status-err)'
    case ButtonTone.Warn:
      return 'var(--theme-status-warn)'
    case ButtonTone.Neutral:
    default:
      return 'var(--theme-fg-ink-2)'
  }
})
</script>

<template>
  <button type="button" class="button" :class="[`is-${variant}`, `is-tone-${tone}`]" :disabled="disabled" :style="{ '--tone': toneVar }" @click="(ev) => emit('click', ev)">
    <slot />
  </button>
</template>

<style scoped>
@reference '../assets/styles.css';

.button {
  @apply inline-flex items-center gap-1 border px-2 py-[2px] text-[0.75rem] leading-tight transition-colors;
  font-family: var(--theme-font-mono);
  color: var(--tone);
  border-color: var(--tone);
  background-color: transparent;
}

.button.is-solid {
  color: var(--theme-fg-on-tone);
  background-color: var(--tone);
}

.button:hover:not(:disabled) {
  background-color: var(--tone);
  color: var(--theme-fg-on-tone);
}

.button.is-solid:hover:not(:disabled) {
  filter: brightness(1.15);
}

.button:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>
