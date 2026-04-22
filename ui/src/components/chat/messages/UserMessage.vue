<script setup lang="ts">
defineProps<{
  text: string
}>()
</script>

<template>
  <article class="user-message" data-testid="user-message">
    <header class="user-message-header">
      <span class="user-message-dot" aria-hidden="true" />
      <span class="user-message-label">you</span>
    </header>
    <p class="user-message-body">{{ text }}</p>
  </article>
</template>

<style scoped>
@reference "../../../assets/styles.css";

.user-message {
  @apply flex flex-col gap-1 border px-3 py-2 text-[0.9rem];
  background-color: var(--theme-surface-card-user);
  border-color: var(--theme-border-soft);
  color: var(--theme-fg);
}

.user-message-header {
  @apply flex items-center gap-2 text-[0.75rem] uppercase tracking-wider;
  color: var(--theme-accent-user);
}

.user-message-dot {
  @apply h-2 w-2 rounded-full;
  background-color: var(--theme-accent-user);
}

.user-message-label {
  @apply font-bold;
}

.user-message-body {
  @apply m-0 whitespace-pre-wrap leading-snug;
  font-family: var(--theme-font-family);
}
</style>
