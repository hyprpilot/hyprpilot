<script setup lang="ts">
import { computed } from 'vue'

import { Role } from '../types'

const props = defineProps<{
  role: Role
  label: string
}>()

const glyph = computed(() => props.label.charAt(0).toUpperCase())
const tone = computed(() => (props.role === Role.User ? 'var(--theme-accent-user)' : 'var(--theme-accent-assistant)'))
</script>

<template>
  <div class="role-tag" :data-role="role" :style="{ '--tone': tone }">
    <span class="role-tag-square" aria-hidden="true">{{ glyph }}</span>
    <span class="role-tag-label">{{ label }}</span>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.role-tag {
  @apply inline-flex items-center gap-1 text-[0.7rem] leading-tight;
  color: var(--tone);
  font-family: var(--theme-font-mono);
  text-transform: lowercase;
}

.role-tag-square {
  @apply inline-flex h-4 w-4 items-center justify-center text-[0.68rem] font-bold;
  color: var(--theme-surface-bg);
  background-color: var(--tone);
}

.role-tag-label {
  @apply font-bold tracking-wider;
}
</style>
