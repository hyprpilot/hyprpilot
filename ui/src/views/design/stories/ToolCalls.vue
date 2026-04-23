<script setup lang="ts">
import { bashDone, planItems, planningCard, smallTools, terminal, thinkingCard, toolCallsFrame, writeDoneDiff } from './tool-calls.fixture'
import { ChatComposer, Frame, ChatStreamCard, ChatTerminalCard, ChatToolChips, ChatToolRowBig } from '@components'
</script>

<template>
  <Frame v-bind="toolCallsFrame">
    <div class="tool-calls-body">
      <ChatStreamCard v-bind="thinkingCard" />
      <ChatStreamCard v-bind="planningCard" :items="planItems" />
      <ChatToolChips :items="smallTools" />

      <ChatToolRowBig :item="bashDone" />
      <ChatTerminalCard v-bind="terminal" />

      <div class="tool-calls-write-row">
        <span class="tool-calls-write-label">Write</span>
        <span class="tool-calls-write-arg">tools/fs.rs</span>
        <span class="tool-calls-write-spacer" />
        <span class="tool-calls-write-diff">
          <span class="tool-calls-write-added">+{{ writeDoneDiff.added }}</span>
          <span class="tool-calls-write-sep"> / </span>
          <span class="tool-calls-write-removed">−{{ writeDoneDiff.removed }}</span>
        </span>
      </div>
    </div>

    <template #composer>
      <ChatComposer />
    </template>
  </Frame>
</template>

<style scoped>
@reference '../../../assets/styles.css';

.tool-calls-body {
  @apply flex flex-col gap-2 px-[14px];
}

.tool-calls-write-row {
  @apply flex items-center gap-2 border-l-[3px] px-[10px] py-[4px] text-[0.62rem];
  font-family: var(--theme-font-mono);
  background-color: var(--theme-surface);
  border-color: var(--theme-fg-dim);
  border-top: 1px solid var(--theme-border);
  border-right: 1px solid var(--theme-border);
  border-bottom: 1px solid var(--theme-border);
}

.tool-calls-write-label {
  @apply shrink-0 font-bold;
  color: var(--theme-fg-dim);
  min-width: 36px;
}

.tool-calls-write-arg {
  color: var(--theme-fg-ink-2);
}

.tool-calls-write-spacer {
  @apply flex-1;
}

.tool-calls-write-added {
  color: var(--theme-status-ok);
}

.tool-calls-write-sep {
  color: var(--theme-fg-dim);
}

.tool-calls-write-removed {
  color: var(--theme-status-err);
}
</style>
