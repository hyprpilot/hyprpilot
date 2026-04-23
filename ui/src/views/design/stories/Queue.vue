<script setup lang="ts">
import { bashDone, messages, queueFrame, thinkingCard, toast } from './queue.fixture'
import { ChatComposer, Frame, ChatQueueStrip, ChatStreamCard, Toast, ChatToolChips, ToastTone } from '@components'
</script>

<template>
  <Frame v-bind="queueFrame">
    <template #toast>
      <Toast :tone="ToastTone.Ok" :message="toast.message" />
    </template>

    <div class="queue-body">
      <ChatToolChips :items="[bashDone]" />
      <ChatStreamCard v-bind="thinkingCard" />
    </div>

    <ChatQueueStrip :messages="messages" />

    <template #composer>
      <ChatComposer />
    </template>
  </Frame>
</template>

<style scoped>
@reference '../../../assets/styles.css';

.queue-body {
  @apply flex flex-col gap-1 px-[14px];
}
</style>
