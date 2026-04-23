<script setup lang="ts">
import { assistantSummary, permissionFrame, prompts, userPrompt } from './permission.fixture'
import { ChatComposer, Frame, ChatPermissionStack, ChatStreamCard, ChatTurn, ChatUserBody, Role, StreamKind } from '@components'
</script>

<template>
  <Frame v-bind="permissionFrame">
    <div class="permission-body">
      <ChatTurn :role="Role.User">
        <ChatUserBody>{{ userPrompt }}</ChatUserBody>
      </ChatTurn>
      <ChatTurn :role="Role.Assistant" elapsed="0.4s">
        <ChatStreamCard :kind="StreamKind.Thinking" :active="false" label="thought" elapsed="0.4s" :summary="assistantSummary" />
      </ChatTurn>
    </div>

    <ChatPermissionStack :prompts="prompts" />

    <template #composer>
      <ChatComposer />
    </template>
  </Frame>
</template>

<style scoped>
@reference '../../../assets/styles.css';

.permission-body {
  @apply flex flex-col gap-1 px-[14px];
}
</style>
