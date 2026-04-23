<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'

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
  type PermissionPrompt,
  type PlanItem
} from '@components'
import {
  pushTranscriptChunk,
  StreamItemKind,
  toView,
  TurnRole,
  useActiveInstance,
  useAdapter,
  useProfiles,
  useSessionHistory,
  useStream,
  useTools,
  useTranscript,
  startSessionStream,
  type PermissionRequestEvent,
  type PlanEntry
} from '@composables'

const { lastPermission, bind, submit } = useAdapter()
const { profiles, selected: selectedProfile } = useProfiles()
const activeAgentId = computed(() => profiles.value.find((p) => p.id === selectedProfile.value)?.agent)
// Session history is wired but the new Frame/Chat primitives don't
// surface a session picker yet — keeping the binding live so the
// backend stays warm; the palette view (K-249) takes over this role.
useSessionHistory(activeAgentId, selectedProfile)

const { id: activeInstanceId } = useActiveInstance()
const { turns } = useTranscript()
const { items: streamItems } = useStream()
const { calls: toolCalls } = useTools()

const sending = ref(false)
const lastErr = ref<string>()
const composerRef = ref<InstanceType<typeof ChatComposer>>()

const activeProfile = computed(() => profiles.value.find((p) => p.id === selectedProfile.value))
const permissionPrompts = computed(() => permissionPromptsFrom(lastPermission.value))

// One interleaved timeline so thoughts / plans / tool calls render
// between the text turns they originally arrived between — not in
// trailing lumps. Sort by the shared createdAt (per-instance
// monotonic seq); ties stable-sorted by kind for determinism.
const KIND_ORDER = { turn: 0, stream: 1, tool: 2 } as const
interface TimelineTurn {
  kind: 'turn'
  createdAt: number
  turn: (typeof turns.value)[number]
}
interface TimelineStream {
  kind: 'stream'
  createdAt: number
  item: (typeof streamItems.value)[number]
}
interface TimelineTool {
  kind: 'tool'
  createdAt: number
  call: (typeof toolCalls.value)[number]
}
type TimelineEntry = TimelineTurn | TimelineStream | TimelineTool

// A "block" is a run of consecutive timeline entries that share a
// speaker role. Each block renders as ONE <ChatTurn> — header + role
// tag once, continuous colored left border across every child
// (thoughts, plans, tool chips, message body). The role flips when
// the next entry belongs to the other speaker.
interface TimelineBlock {
  role: Role
  startedAt: number
  entries: TimelineEntry[]
}

function roleFor(entry: TimelineEntry): Role {
  if (entry.kind === 'turn') {
    return entry.turn.role === TurnRole.User ? Role.User : Role.Assistant
  }

  return Role.Assistant
}

const timelineBlocks = computed<TimelineBlock[]>(() => {
  const entries: TimelineEntry[] = [
    ...turns.value.map<TimelineTurn>((turn) => ({ kind: 'turn', createdAt: turn.createdAt, turn })),
    ...streamItems.value.map<TimelineStream>((item) => ({ kind: 'stream', createdAt: item.createdAt, item })),
    ...toolCalls.value.map<TimelineTool>((call) => ({ kind: 'tool', createdAt: call.createdAt, call }))
  ]
  entries.sort((a, b) => a.createdAt - b.createdAt || KIND_ORDER[a.kind] - KIND_ORDER[b.kind])

  const blocks: TimelineBlock[] = []
  for (const entry of entries) {
    const role = roleFor(entry)
    const last = blocks[blocks.length - 1]
    if (last && last.role === role) {
      last.entries.push(entry)
    } else {
      blocks.push({ role, startedAt: entry.createdAt, entries: [entry] })
    }
  }

  return blocks
})

let stopStream: (() => void) | undefined

onMounted(async () => {
  try {
    stopStream = await startSessionStream()
  } catch (err) {
    lastErr.value = `stream bind failed: ${String(err)}`
  }
  bind().catch((err) => {
    lastErr.value = `bind failed: ${String(err)}`
  })
})

onUnmounted(() => {
  stopStream?.()
  stopStream = undefined
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
  submit({ text, profileId: selectedProfile.value })
    .then((result) => {
      // Agents don't echo user prompts on session/prompt (only on
      // session/load replay), so push the user turn locally using the
      // instance/session ids the daemon just handed back. This lands
      // the captain bubble in the transcript immediately.
      const instanceId = result.instance_id ?? activeInstanceId.value
      const sessionId = result.session_id ?? ''
      if (instanceId) {
        pushTranscriptChunk(instanceId, sessionId, {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text }
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
    <div class="chat-transcript" data-testid="chat-transcript" :data-instance-id="activeInstanceId ?? ''">
      <ChatTurn v-for="block in timelineBlocks" :key="`${block.role}-${block.startedAt}`" :role="block.role">
        <template v-for="entry in block.entries" :key="`${entry.kind}-${entry.createdAt}`">
          <ChatUserBody v-if="entry.kind === 'turn' && entry.turn.role === TurnRole.User">{{ entry.turn.text }}</ChatUserBody>
          <ChatAssistantBody v-else-if="entry.kind === 'turn'">{{ entry.turn.text }}</ChatAssistantBody>

          <ChatStreamCard
            v-else-if="entry.kind === 'stream' && entry.item.kind === StreamItemKind.Thought"
            :kind="StreamKind.Thinking"
            :active="true"
            label="thought"
            >{{ entry.item.text }}</ChatStreamCard
          >
          <ChatStreamCard
            v-else-if="entry.kind === 'stream'"
            :kind="StreamKind.Planning"
            :active="true"
            label="plan"
            :items="mapPlanItems(entry.item.entries)"
          />

          <ChatToolChips v-else-if="entry.kind === 'tool'" :items="[toView(entry.call)]" />
        </template>
      </ChatTurn>
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
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
}

.chat-err {
  @apply px-3 py-1 text-[0.8rem];
  color: var(--theme-status-err);
  background-color: var(--theme-surface);
}
</style>
