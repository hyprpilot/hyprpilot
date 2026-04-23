<script setup lang="ts">
import { computed } from 'vue'

import { ToastTone, type FaIconSpec } from './types'

const props = withDefaults(
  defineProps<{
    tone?: ToastTone
    message: string
    dismissible?: boolean
  }>(),
  { tone: ToastTone.Ok, dismissible: true }
)

const emit = defineEmits<{
  dismiss: []
}>()

const toneIcon = computed<FaIconSpec>(() => {
  switch (props.tone) {
    case ToastTone.Ok:
      return ['fas', 'circle-check']
    case ToastTone.Warn:
      return ['fas', 'triangle-exclamation']
    case ToastTone.Err:
      return ['fas', 'circle-xmark']
    default:
      return ['fas', 'circle-info']
  }
})

const toneVar = computed(() => {
  switch (props.tone) {
    case ToastTone.Ok:
      return 'var(--theme-status-ok)'
    case ToastTone.Warn:
      return 'var(--theme-status-warn)'
    case ToastTone.Err:
      return 'var(--theme-status-err)'
    default:
      return 'var(--theme-fg-dim)'
  }
})

const ariaRole = computed(() => (props.tone === ToastTone.Err ? 'alert' : 'status'))
</script>

<template>
  <div class="toast" :class="`is-${tone}`" :role="ariaRole" :style="{ '--tone': toneVar }">
    <FaIcon :icon="toneIcon" class="toast-tone-icon" aria-hidden="true" />
    <span class="toast-message">{{ message }}</span>
    <button v-if="dismissible" type="button" class="toast-dismiss" aria-label="dismiss" @click="emit('dismiss')">
      <FaIcon :icon="['fas', 'xmark']" class="toast-dismiss-icon" />
    </button>
  </div>
</template>

<style scoped>
@reference '../assets/styles.css';

.toast {
  @apply flex items-center gap-2 border-l-[3px] px-3 py-[6px] text-[0.75rem] leading-tight;
  font-family: var(--theme-font-mono);
  border-color: var(--tone);
  color: var(--theme-fg-ink-2);
  background-color: var(--theme-surface-alt);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
}

.toast-tone-icon {
  @apply shrink-0;
  width: 12px;
  height: 12px;
  color: var(--tone);
}

.toast-message {
  @apply flex-1;
}

.toast-dismiss {
  @apply shrink-0 border-0 bg-transparent px-1 text-[0.8rem] leading-none;
  color: var(--theme-fg-dim);
  cursor: pointer;
}

.toast-dismiss-icon {
  width: 10px;
  height: 10px;
}

.toast-dismiss:hover {
  color: var(--theme-fg-ink-2);
}
</style>
