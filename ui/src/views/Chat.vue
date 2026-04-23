<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import {
  ChatAssistantBody,
  ChatComposer,
  ChatPermissionStack,
  ChatStreamCard,
  ChatToolChips,
  ChatTurn,
  ChatUserBody,
  Frame,
  PlanStatus,
  Role,
  StreamKind,
  ToolState,
  type PermissionPrompt,
  type PlanItem,
  type ToolChipItem
} from '@components'
import {
  EventKind,
  MessageKind,
  useAdapter,
  useProfiles,
  useSessionHistory,
  useTranscript,
  type ChatMessage,
  type ContentBlock,
  type PlanEntry,
  type PermissionRequestEvent
} from '@composables'

const { transcript, state, lastPermission, bind, submit } = useAdapter()
const { profiles, selected: selectedProfile } = useProfiles()
const activeAgentId = computed(() => profiles.value.find((p) => p.id === selectedProfile.value)?.agent)
// Session history is wired but the new Frame/Chat primitives don't
// surface a session picker yet — keeping the binding live so the
// backend stays warm; the palette view (K-249) takes over this role.
useSessionHistory(activeAgentId, selectedProfile)
const { messages } = useTranscript(transcript)

const sending = ref(false)
const lastErr = ref<string>()
const composerRef = ref<InstanceType<typeof ChatComposer>>()

const activeProfile = computed(() => profiles.value.find((p) => p.id === selectedProfile.value))
const permissionPrompts = computed(() => permissionPromptsFrom(lastPermission.value))

onMounted(() => {
  bind().catch((err) => {
    lastErr.value = `bind failed: ${String(err)}`
  })
})

function mapPlanStatus(raw?: string): PlanStatus {
  switch (raw) {
    case 'completed':
      return PlanStatus.Completed
    case 'in_progress':
      return PlanStatus.InProgress
    default:
      return PlanStatus.Pending
  }
}

function mapPlanItems(entries: PlanEntry[]): PlanItem[] {
  return entries.map((e) => ({ status: mapPlanStatus(e.status), text: e.content ?? '' }))
}

function mapToolStatus(raw?: string): ToolState {
  switch (raw) {
    case 'completed':
    case 'done':
      return ToolState.Done
    case 'failed':
    case 'error':
      return ToolState.Failed
    case 'awaiting':
    case 'pending':
      return ToolState.Awaiting
    default:
      return ToolState.Running
  }
}

// Short-form of `rawInput` for the chip body: join the first few
// key/value pairs, truncate long values. Keeps the chip reading at a
// glance without re-flowing for big payloads.
function shortArg(raw: Record<string, unknown>): string | undefined {
  const parts: string[] = []
  for (const [k, v] of Object.entries(raw)) {
    let rendered: string
    if (typeof v === 'string') {
      rendered = v
    } else {
      try {
        rendered = JSON.stringify(v)
      } catch {
        rendered = String(v)
      }
    }
    if (rendered.length > 60) {
      rendered = `${rendered.slice(0, 57)}...`
    }
    parts.push(`${k}=${rendered}`)
    if (parts.length >= 3) {
      break
    }
  }

  return parts.length > 0 ? parts.join(' ') : undefined
}

function pickLastOutput(content: ContentBlock[]): string | undefined {
  for (let i = content.length - 1; i >= 0; i -= 1) {
    const block = content[i]
    if (block && typeof block.text === 'string' && block.text.length > 0) {
      const first = block.text.split('\n', 1)[0] ?? ''

      return first.length > 120 ? `${first.slice(0, 117)}...` : first
    }
  }

  return undefined
}

// useTranscript merges tool-call chunks into one AgentToolCall per
// toolCallId; Chat.vue wraps each in a ChatToolChips (which itself
// handles small-vs-big tool dispatch by label).
function toolChipFor(msg: Extract<ChatMessage, { kind: MessageKind.AgentToolCall }>): ToolChipItem {
  const { call } = msg
  const stat = call.locations?.find((l) => typeof l.path === 'string' && l.path.length > 0)?.path

  return {
    label: call.title ?? call.toolCallId,
    kind: call.kind,
    state: mapToolStatus(call.status),
    arg: call.rawInput ? shortArg(call.rawInput) : undefined,
    detail: pickLastOutput(call.content),
    stat
  }
}

function permissionPromptsFrom(ev?: PermissionRequestEvent): PermissionPrompt[] {
  if (!ev) {
    return []
  }
  // ACP delivers N option ids per request; ChatPermissionStack renders
  // allow/deny for the oldest active. Until the PermissionController
  // (K-6) surfaces richer metadata, reuse the session id as the row
  // identity and print the option summary as the args body.
  const args = ev.options.map((o) => o.option_id).join(' · ')

  return [{ id: ev.session_id, tool: 'permission', kind: 'acp', args }]
}

function onSubmit(text: string): void {
  sending.value = true
  lastErr.value = undefined
  // Agents don't echo user prompts on session/prompt — they only replay
  // user_message_chunk during session/load. Inject the local bubble so
  // the user sees their own submit. The first submit CREATES the session
  // (Rust side runs session/new on demand), so session_id is unknown
  // until submit() resolves — we push the bubble against the returned
  // id rather than fabricating a placeholder.
  submit({ text, profileId: selectedProfile.value })
    .then((result) => {
      const sessionId = result.session_id ?? state.value?.session_id
      if (sessionId) {
        transcript.push({
          kind: EventKind.Transcript,
          agent_id: activeAgentId.value ?? '',
          session_id: sessionId,
          update: { sessionUpdate: 'user_message_chunk', content: { type: 'text', text } }
        })
      }
      composerRef.value?.clear()
    })
    .catch((err) => {
      lastErr.value = String(err)
    })
    .finally(() => {
      sending.value = false
    })
}
</script>

<template>
  <Frame :profile="selectedProfile ?? 'none'" :provider="activeProfile?.agent" :model="activeProfile?.model">
    <div class="chat-transcript" data-testid="chat-transcript">
      <template v-for="msg in messages" :key="msg.id">
        <ChatTurn v-if="msg.kind === MessageKind.User" :role="Role.User">
          <ChatUserBody>{{ msg.text }}</ChatUserBody>
        </ChatTurn>

        <ChatTurn v-else-if="msg.kind === MessageKind.AgentMessage" :role="Role.Assistant">
          <ChatAssistantBody>{{ msg.text }}</ChatAssistantBody>
        </ChatTurn>

        <ChatTurn v-else-if="msg.kind === MessageKind.AgentThought" :role="Role.Assistant">
          <ChatStreamCard :kind="StreamKind.Thinking" :active="true" label="thought">{{ msg.text }}</ChatStreamCard>
        </ChatTurn>

        <ChatTurn v-else-if="msg.kind === MessageKind.AgentPlan" :role="Role.Assistant">
          <ChatStreamCard :kind="StreamKind.Planning" :active="true" label="plan" :items="mapPlanItems(msg.entries)" />
        </ChatTurn>

        <ChatTurn v-else-if="msg.kind === MessageKind.AgentToolCall" :role="Role.Assistant">
          <ChatToolChips :items="[toolChipFor(msg)]" />
        </ChatTurn>
      </template>
    </div>

    <ChatPermissionStack :prompts="permissionPrompts" />

    <p v-if="lastErr" class="chat-err" data-testid="chat-err">{{ lastErr }}</p>

    <template #composer>
      <ChatComposer ref="composerRef" :sending="sending" @submit="onSubmit" />
    </template>
  </Frame>
</template>

<style scoped>
@reference '../assets/styles.css';

.chat-transcript {
  @apply flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto px-[14px] py-2;
}

.chat-err {
  @apply px-3 py-1 text-[0.8rem];
  color: var(--theme-status-err);
  background-color: var(--theme-surface);
}
</style>
