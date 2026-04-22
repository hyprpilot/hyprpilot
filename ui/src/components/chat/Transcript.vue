<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'

import AgentMessage from './messages/AgentMessage.vue'
import AgentPlan from './messages/AgentPlan.vue'
import AgentThought from './messages/AgentThought.vue'
import AgentToolCall from './messages/AgentToolCall.vue'
import UserMessage from './messages/UserMessage.vue'
import { type ChatMessage, MessageKind } from '@composables'

const props = defineProps<{
  messages: ChatMessage[]
}>()

const scroller = ref<HTMLElement>()
const followTail = ref(true)

function isAtBottom(el: HTMLElement): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight < 24
}

function onScroll(): void {
  const el = scroller.value
  if (!el) {
    return
  }
  followTail.value = isAtBottom(el)
}

// Streaming chunks merge into the tail bubble in place, so `length` alone
// doesn't tick — pair it with `last.updatedAt` (bumped on every merge).
watch(
  () => {
    const last = props.messages[props.messages.length - 1]

    return [props.messages.length, last?.updatedAt] as const
  },
  async () => {
    if (!followTail.value) {
      return
    }
    await nextTick()
    const el = scroller.value
    if (el) {
      el.scrollTop = el.scrollHeight
    }
  }
)
</script>

<template>
  <section ref="scroller" class="transcript" data-testid="transcript" @scroll="onScroll">
    <p v-if="messages.length === 0" class="transcript-empty" data-testid="transcript-empty">no messages yet — type something below.</p>

    <template v-for="msg in messages" :key="msg.id">
      <UserMessage v-if="msg.kind === MessageKind.User" :text="msg.text" />
      <AgentMessage v-else-if="msg.kind === MessageKind.AgentMessage" :text="msg.text" />
      <AgentThought v-else-if="msg.kind === MessageKind.AgentThought" :text="msg.text" />
      <AgentToolCall v-else-if="msg.kind === MessageKind.AgentToolCall" :call="msg.call" />
      <AgentPlan v-else-if="msg.kind === MessageKind.AgentPlan" :entries="msg.entries" />
    </template>
  </section>
</template>

<style scoped>
@reference "../../assets/styles.css";

.transcript {
  @apply flex flex-1 flex-col gap-2 overflow-y-auto px-2 py-2;
  min-height: 0;
}

.transcript-empty {
  @apply m-auto text-[0.85rem];
  color: var(--theme-fg-muted);
  font-family: var(--theme-font-family);
}
</style>
