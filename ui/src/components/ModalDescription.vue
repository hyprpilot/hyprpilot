<script setup lang="ts">
/**
 * Modal body primitive — dim explanatory paragraph that sits above
 * an interactive body (input, code editor, picker). Composable inside
 * `<Modal>`'s default slot alongside other primitives like
 * `<ModalInput>` / `<ModalMarkdownBody>` / `<ModalPreBody>`.
 *
 * Plain text is the common case (rendered via the default slot);
 * pass arbitrary children when the description needs inline
 * formatting (bold, code spans). Per CLAUDE.md compose-not-bag
 * rule — no `text: string` prop bag.
 */
</script>

<template>
  <p class="modal-description">
    <slot />
  </p>
</template>

<style scoped>
@reference '../assets/styles.css';

.modal-description {
  margin: 0 0 12px 0;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
  line-height: 1.5;
}

.modal-description :deep(code) {
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
  padding: 1px 4px;
  border-radius: 2px;
  font-size: 0.65rem;
}
</style>
