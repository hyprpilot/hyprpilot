<script setup lang="ts">
import { computed } from 'vue'

import { Role } from '../types'

const props = defineProps<{
  role: Role
}>()

const accent = computed(() => (props.role === Role.User ? 'var(--theme-accent-user)' : 'var(--theme-accent-assistant)'))
</script>

<template>
  <div class="chat-body" :data-role="role" :style="{ '--accent': accent }">
    <slot />
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.chat-body {
  @apply border-l-[3px] px-3 py-2 text-[0.9rem] leading-snug;
  color: var(--theme-fg);
  background-color: var(--theme-surface);
  border-color: var(--accent);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
  overflow-wrap: anywhere;
  min-width: 0;
  font-family: var(--theme-font-sans);
}

.chat-body[data-role='user'] {
  white-space: pre-wrap;
}
</style>
