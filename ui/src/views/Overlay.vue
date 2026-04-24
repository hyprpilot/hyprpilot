<script setup lang="ts">
/**
 * Overlay shell — the single page-level view the Tauri webview mounts
 * (see `App.vue`). Composes the K-250 chat primitives into the running app.
 *
 * Frame slots (see `components/Frame.vue`):
 *   default slot  — transcript body. `<ChatTurn>` blocks built from
 *                   `useTranscript` + `useStream` + `useTools`, followed
 *                   by `<ChatPermissionStack>` fed from
 *                   `useAdapter().lastPermission`.
 *   #composer     — `<ChatComposer>` wired to `useAdapter().submit`.
 *   #toast        — unused today; reserved for a future toast surface.
 *
 * Header rows 1 + 2 are driven by Frame props (profile, modeTag, provider,
 * model, title, cwd, gitStatus, counts) — no named slots for the header.
 *
 * State sources (all from `@composables`):
 *   useAdapter          → bind / submit / lastPermission
 *   useProfiles         → profile registry + selected profile
 *   useSessionHistory   → warms the session store for the palette (K-249)
 *   useTranscript       → user/assistant turns
 *   useStream           → thought / plan stream items
 *   useTools            → tool-call records for the inline chip row
 *   useActiveInstance   → current instance id for the transcript data-attr
 *   startSessionStream  → starts the demuxed Tauri event pump
 */
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
  Toast,
  ToastTone,
  type PlanItem
} from '@components'
import {
  pushToast,
  pushTranscriptChunk,
  StreamItemKind,
  toView,
  TurnRole,
  useActiveInstance,
  useAdapter,
  usePermissions,
  usePhase,
  useProfiles,
  useSessionHistory,
  useStream,
  useToasts,
  useTools,
  useTranscript,
  startSessionStream,
  type PlanEntry
} from '@composables'
import { log } from '@lib'

const { submit } = useAdapter()
const { entries: toasts, dismiss } = useToasts()
const { phase } = usePhase()
const { profiles, selected: selectedProfile } = useProfiles()
const activeAgentId = computed(() => profiles.value.find((p) => p.id === selectedProfile.value)?.agent)
// Session history is wired but the overlay shell doesn't surface a
// session picker yet — keeping the binding live so the backend stays
// warm; the palette view (K-249) takes over this role.
useSessionHistory(activeAgentId, selectedProfile)

const { id: activeInstanceId } = useActiveInstance()
const { turns } = useTranscript()
const { items: streamItems } = useStream()
const { calls: toolCalls } = useTools()
const { pending: permissionPrompts, allow, deny } = usePermissions()

const sending = ref(false)
const composerRef = ref<InstanceType<typeof ChatComposer>>()

const activeProfile = computed(() => profiles.value.find((p) => p.id === selectedProfile.value))

// Timeline is interleaved only to determine block boundaries —
// consecutive entries that share a speaker role collapse into one
// `<ChatTurn>`. Within an assistant block, entries get split into three
// buckets so the final render order is: thoughts + plans → tool-call
// grid → assistant reply body. User blocks carry just a body.
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

interface TimelineBlock {
  role: Role
  startedAt: number
  streamEntries: TimelineStream[]
  toolCalls: TimelineTool[]
  turnEntries: TimelineTurn[]
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
    const block =
      last && last.role === role
        ? last
        : (blocks.push({ role, startedAt: entry.createdAt, streamEntries: [], toolCalls: [], turnEntries: [] }),
          blocks[blocks.length - 1])
    if (entry.kind === 'stream') {
      block.streamEntries.push(entry)
    } else if (entry.kind === 'tool') {
      block.toolCalls.push(entry)
    } else {
      block.turnEntries.push(entry)
    }
  }

  return blocks
})

let stopStream: (() => void) | undefined

function isEditableTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) {
    return false
  }
  const tag = el.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA') {
    return true
  }

  return el.isContentEditable
}

function onKeydown(event: KeyboardEvent): void {
  // TODO(K-281 follow-up): Tab = next row cycling. Today keyboard
  // a/d always addresses the oldest-active (first non-queued) prompt.
  // Multi-row focus navigation lands with the ChatPermissionStack
  // focus-index contract (requires the primitive to expose a
  // :focused-index prop + :focus event).
  if (event.repeat || event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) {
    return
  }
  if (event.key !== 'a' && event.key !== 'd') {
    return
  }
  if (isEditableTarget(event.target)) {
    return
  }
  const active = permissionPrompts.value.find((p) => !p.queued) ?? permissionPrompts.value[0]
  if (!active) {
    return
  }
  event.preventDefault()
  log.info('keybind invoked', { key: event.key, target: 'permission' })
  if (event.key === 'a') {
    onAllow(active.requestId)
  } else {
    onDeny(active.requestId)
  }
}

onMounted(async () => {
  try {
    stopStream = await startSessionStream()
  } catch (err) {
    log.error('invoke failed', { command: 'startSessionStream' }, err)
    pushToast(ToastTone.Err, `stream bind failed: ${String(err)}`)
  }
  document.addEventListener('keydown', onKeydown)
})

onUnmounted(() => {
  stopStream?.()
  stopStream = undefined
  document.removeEventListener('keydown', onKeydown)
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

async function onAllow(requestId: string): Promise<void> {
  log.info('permission click', { choice: 'allow', requestId })
  try {
    await allow(requestId)
  } catch (err) {
    log.error('invoke failed', { command: 'permission_reply', choice: 'allow', requestId }, err)
    pushToast(ToastTone.Err, `allow failed: ${String(err)}`)
  }
}

async function onDeny(requestId: string): Promise<void> {
  log.info('permission click', { choice: 'deny', requestId })
  try {
    await deny(requestId)
  } catch (err) {
    log.error('invoke failed', { command: 'permission_reply', choice: 'deny', requestId }, err)
    pushToast(ToastTone.Err, `deny failed: ${String(err)}`)
  }
}

function onSubmit(text: string): void {
  log.info('composer submit', { text_len: text.length, profileId: selectedProfile.value })
  sending.value = true
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
      log.error('invoke failed', { command: 'session_submit' }, err)
      pushToast(ToastTone.Err, String(err))
    })
    .finally(() => {
      sending.value = false
    })
}
</script>

<template>
  <Frame :profile="selectedProfile ?? 'none'" :phase="phase" :provider="activeProfile?.agent" :model="activeProfile?.model">
    <div class="chat-transcript" data-testid="chat-transcript" :data-instance-id="activeInstanceId ?? ''">
      <ChatTurn v-for="block in timelineBlocks" :key="`${block.role}-${block.startedAt}`" :role="block.role">
        <template v-for="entry in block.streamEntries" :key="`stream-${entry.createdAt}`">
          <ChatStreamCard
            v-if="entry.item.kind === StreamItemKind.Thought"
            :kind="StreamKind.Thinking"
            :active="true"
            label="thought"
            >{{ entry.item.text }}</ChatStreamCard
          >
          <ChatStreamCard
            v-else-if="entry.item.kind === StreamItemKind.Plan"
            :kind="StreamKind.Planning"
            :active="true"
            label="plan"
            :items="mapPlanItems(entry.item.entries)"
          />
        </template>

        <ChatToolChips
          v-if="block.toolCalls.length > 0"
          :items="block.toolCalls.map((t) => toView(t.call))"
          grouped
        />

        <template v-for="entry in block.turnEntries" :key="`turn-${entry.createdAt}`">
          <ChatUserBody v-if="entry.turn.role === TurnRole.User">{{ entry.turn.text }}</ChatUserBody>
          <ChatAssistantBody v-else>{{ entry.turn.text }}</ChatAssistantBody>
        </template>
      </ChatTurn>
    </div>

    <ChatPermissionStack :prompts="permissionPrompts" @allow="onAllow" @deny="onDeny" />

    <template #toast>
      <div v-if="toasts.length > 0" class="toast-stack">
        <Toast v-for="t in toasts" :key="t.id" :tone="t.tone" :message="t.message" @dismiss="dismiss(t.id)" />
      </div>
    </template>

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

.toast-stack {
  @apply flex flex-col gap-1;
}
</style>
