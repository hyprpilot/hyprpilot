<script setup lang="ts">
defineProps<{ source: string }>()
</script>

<template>
  <pre class="text-body">{{ source }}</pre>
</template>

<style scoped>
@reference '../assets/styles.css';

.text-body {
  @apply m-0 overflow-x-auto;
  font-family: var(--theme-font-mono);
  font-size: 0.78rem;
  line-height: 1.4;
  white-space: pre-wrap;
  color: var(--theme-fg);
}
</style>
