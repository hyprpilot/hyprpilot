<script setup lang="ts">
import { computed } from 'vue'

import { Role } from '@components'

/**
 * role tag — tiny mono uppercase pill with the role color on a
 * tinted-black ("soft") background. Used inside the Turn lane header
 * to mark each block's role at a glance. Captain green, Pilot red.
 */
const props = defineProps<{
  role: Role
  label: string
}>()

const tone = computed(() => (props.role === Role.User ? 'var(--theme-accent-user)' : 'var(--theme-accent-assistant)'))
const soft = computed(() => (props.role === Role.User ? 'var(--theme-accent-user-soft)' : 'var(--theme-accent-assistant-soft)'))
</script>

<template>
  <span class="role-tag" :data-role="role" :style="{ '--tone': tone, '--soft': soft }">
    {{ label }}
  </span>
</template>

<style scoped>
@reference '../../assets/styles.css';

.role-tag {
  @apply inline-flex shrink-0 items-center font-bold leading-tight;
  color: var(--tone);
  background-color: var(--soft);
  font-family: var(--theme-font-mono);
  text-transform: lowercase;
  padding: 1px 7px;
  border-radius: 3px;
  font-size: 0.62rem;
  letter-spacing: 0.4px;
}
</style>
