<script setup lang="ts">
import { idleFrame, liveSessions } from './idle.fixture'
import { ChatComposer, ChatLiveSessionRow, Frame } from '@components'
</script>

<template>
  <Frame v-bind="idleFrame">
    <section class="idle-body">
      <div class="idle-wordmark">hyprpilot</div>
      <div class="idle-tagline">LFG.</div>

      <div class="idle-hints">
        <span class="idle-hints-key">Ctrl+K</span><span class="idle-hints-desc">command palette</span> <span class="idle-hints-key">@</span
        ><span class="idle-hints-desc">reference a file or folder</span> <span class="idle-hints-key">+</span><span class="idle-hints-desc">attach a skill or reference</span>
        <span class="idle-hints-key">Esc</span><span class="idle-hints-desc">close overlay</span>
      </div>

      <div class="idle-sessions">
        <div class="idle-sessions-count">
          <span class="idle-sessions-count-n">{{ liveSessions.length }}</span>
          <span>sessions live</span>
        </div>
        <ChatLiveSessionRow v-for="s in liveSessions" :key="s.id" :session="s" />
      </div>
    </section>

    <template #composer>
      <ChatComposer :pills="[]" />
    </template>
  </Frame>
</template>

<style scoped>
@reference '../../../assets/styles.css';

.idle-body {
  @apply flex flex-1 flex-col items-center justify-center gap-0 px-6;
  color: var(--theme-fg-dim);
}

.idle-wordmark {
  @apply text-[1.4rem];
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
  font-weight: 500;
  letter-spacing: -0.3px;
}

.idle-tagline {
  @apply mt-1 text-[0.82rem];
  color: var(--theme-accent);
  font-family: var(--theme-font-mono);
  font-weight: 700;
  letter-spacing: 1px;
}

.idle-hints {
  @apply mt-5 grid gap-y-1 gap-x-[10px] text-[0.62rem];
  grid-template-columns: auto 1fr;
  font-family: var(--theme-font-mono);
}

.idle-hints-key {
  color: var(--theme-accent);
}

.idle-hints-desc {
  color: var(--theme-fg-dim);
}

.idle-sessions {
  @apply mt-6 flex flex-col items-center gap-1;
}

.idle-sessions-count {
  @apply inline-flex items-center gap-[6px] border px-2 py-[2px] text-[0.62rem];
  background-color: var(--theme-surface);
  border-color: var(--theme-border-soft);
  color: var(--theme-fg-ink-2);
  font-family: var(--theme-font-mono);
}

.idle-sessions-count-n {
  color: var(--theme-accent);
  font-weight: 700;
}
</style>
