<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import Composer from '@components/chat/Composer.vue'
import SessionList from '@components/chat/SessionList.vue'
import StatusStrip from '@components/chat/StatusStrip.vue'
import Transcript from '@components/chat/Transcript.vue'
import PermissionPrompt from '@components/PermissionPrompt.vue'
import { EventKind, useAcpAgent, useAcpProfiles, useAcpSessionHistory, useAcpTranscript } from '@composables'

const { transcript, state, lastPermission, bind, submit, cancel } = useAcpAgent()
const { profiles, selected: selectedProfile, select: selectProfile } = useAcpProfiles()
const activeAgentId = computed(() => profiles.value.find((p) => p.id === selectedProfile.value)?.agent)
const { sessions, loading: sessionsLoading, load: loadSession } = useAcpSessionHistory(activeAgentId, selectedProfile)
const { messages } = useAcpTranscript(transcript)

const sending = ref(false)
const lastErr = ref<string>()
const activeSessionId = computed(() => state.value?.session_id)
const composerRef = ref<InstanceType<typeof Composer>>()

onMounted(() => {
  bind().catch((err) => {
    lastErr.value = `bind failed: ${String(err)}`
  })
})

async function onSubmit(text: string): Promise<void> {
  sending.value = true
  lastErr.value = undefined
  // Agents don't echo user prompts on session/prompt — they only replay
  // user_message_chunk during session/load. Inject the local bubble so
  // the user sees their own submit immediately.
  transcript.push({
    kind: EventKind.Transcript,
    agent_id: activeAgentId.value ?? '',
    session_id: state.value?.session_id ?? 'local',
    update: { sessionUpdate: 'user_message_chunk', content: { type: 'text', text } }
  })
  try {
    await submit({ text, profileId: selectedProfile.value })
    composerRef.value?.clear()
  } catch (err) {
    lastErr.value = String(err)
  } finally {
    sending.value = false
  }
}

async function onCancel(): Promise<void> {
  try {
    await cancel()
  } catch (err) {
    lastErr.value = String(err)
  }
}

async function onLoadSession(sessionId: string): Promise<void> {
  await loadSession(sessionId)
}
</script>

<template>
  <section class="chat" data-testid="chat">
    <StatusStrip :profiles="profiles" :selected-profile-id="selectedProfile" :state="state" @select-profile="selectProfile" />

    <div class="chat-body">
      <SessionList :sessions="sessions" :loading="sessionsLoading" :active-session-id="activeSessionId" @load="onLoadSession" />

      <div class="chat-main">
        <Transcript :messages="messages" />

        <PermissionPrompt :request="lastPermission" />

        <p v-if="lastErr" class="chat-err" data-testid="chat-err">{{ lastErr }}</p>

        <Composer ref="composerRef" :sending="sending" @submit="onSubmit" @cancel="onCancel" />
      </div>
    </div>
  </section>
</template>

<style scoped>
@reference "../assets/styles.css";

.chat {
  @apply flex h-screen flex-col;
  color: var(--theme-fg);
  background-color: var(--theme-window);
}

.chat-body {
  @apply flex min-h-0 flex-1;
}

.chat-main {
  @apply flex min-h-0 flex-1 flex-col;
}

.chat-err {
  @apply px-3 py-1 text-[0.8rem];
  color: var(--theme-accent);
  background-color: var(--theme-surface-compose);
}
</style>
