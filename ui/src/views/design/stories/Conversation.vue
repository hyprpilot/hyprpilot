<script setup lang="ts">
import { assistantClosing, assistantElapsed, conversationFrame, planItems, planningCard, thinkingCard, thinkingText, tools, userPrompt } from './conversation.fixture'
import { ChatBody, ChatComposer, Frame, ChatStreamCard, ChatToolChips, ChatTurn, Role } from '@components'
</script>

<template>
  <Frame v-bind="conversationFrame">
    <div class="conversation-body">
      <ChatTurn :role="Role.User">
        <ChatBody :role="Role.User">{{ userPrompt }}</ChatBody>
      </ChatTurn>

      <ChatTurn :role="Role.Assistant" :elapsed="assistantElapsed" live>
        <ChatStreamCard v-bind="thinkingCard">{{ thinkingText }}</ChatStreamCard>
        <ChatStreamCard v-bind="planningCard" :items="planItems" />
        <ChatToolChips :items="tools" />
        <ChatBody :role="Role.Assistant">{{ assistantClosing }}</ChatBody>
      </ChatTurn>
    </div>

    <template #composer>
      <ChatComposer />
    </template>
  </Frame>
</template>

<style scoped>
@reference '../../../assets/styles.css';

.conversation-body {
  @apply flex flex-col gap-1 px-[14px];
}
</style>
