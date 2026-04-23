<script setup lang="ts">
import { assistantClosing, assistantElapsed, conversationFrame, planItems, planningCard, thinkingCard, thinkingText, tools, userPrompt } from './conversation.fixture'
import { ChatAssistantBody, ChatComposer, Frame, ChatStreamCard, ChatToolChips, ChatTurn, ChatUserBody, Role } from '@components'
</script>

<template>
  <Frame v-bind="conversationFrame">
    <div class="conversation-body">
      <ChatTurn :role="Role.User">
        <ChatUserBody>{{ userPrompt }}</ChatUserBody>
      </ChatTurn>

      <ChatTurn :role="Role.Assistant" :elapsed="assistantElapsed" live>
        <ChatStreamCard v-bind="thinkingCard">{{ thinkingText }}</ChatStreamCard>
        <ChatStreamCard v-bind="planningCard" :items="planItems" />
        <ChatToolChips :items="tools" />
        <ChatAssistantBody>{{ assistantClosing }}</ChatAssistantBody>
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
