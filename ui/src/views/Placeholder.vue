<script setup lang="ts">
import { Button } from '@ui/button'
</script>

<template>
  <section class="overlay-card placeholder" data-testid="placeholder">
    <header class="placeholder-header">
      <span class="placeholder-dot" aria-hidden="true" />
      <h1 class="placeholder-title">hyprpilot</h1>
    </header>

    <p class="placeholder-body">Scaffold only — overlay, socket protocol, ACP bridge, and MCP catalog land in their own issues.</p>

    <Button variant="accent" data-testid="placeholder-button"> Not implemented </Button>
  </section>
</template>

<style scoped>
.placeholder {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  max-width: 28rem;
  margin: 3rem auto;
  padding: 1.25rem 1.5rem;
  background-color: var(--theme-surface-card-user);
  color: var(--theme-fg);
  border: 1px solid var(--theme-border-soft);
  border-left: 2px solid var(--theme-window-edge);
}

.placeholder-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.placeholder-dot {
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 9999px;
  background-color: var(--theme-state-idle);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--theme-state-idle) 25%, transparent);
}

.placeholder-title {
  font-family: var(--theme-font-family);
  font-size: 1.05rem;
  font-weight: 700;
  letter-spacing: 0.04em;
  color: var(--theme-fg);
}

.placeholder-body {
  color: var(--theme-fg-dim);
  font-size: 0.9rem;
  line-height: 1.4;
}
</style>
