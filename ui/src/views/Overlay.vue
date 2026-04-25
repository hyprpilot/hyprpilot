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
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'

import { openRootPalette, openSkillsPalette } from './palette-root'
import {
  ChatBody,
  ChatComposer,
  ChatPermissionStack,
  ChatStreamCard,
  ChatToolChips,
  ChatTurn,
  CommandPalette,
  Frame,
  PlanStatus,
  Role,
  StreamKind,
  Toast,
  ToastTone,
  type PlanItem
} from '@components'
import {
  isEditableTarget,
  pushToast,
  pushTranscriptChunk,
  StreamItemKind,
  TurnRole,
  useActiveInstance,
  useAdapter,
  useAttachments,
  useKeymap,
  useKeymaps,
  usePalette,
  usePermissions,
  usePhase,
  useProfiles,
  useSessionHistory,
  useStream,
  useToasts,
  useTools,
  useTranscript,
  useTurns,
  type KeymapEntry,
  startSessionStream,
  type InstanceId,
  type PlanEntry
} from '@composables'
import { Modifier } from '@ipc'
import { formatToolCall, log } from '@lib'

const { submit } = useAdapter()
const { pending: pendingAttachments, clear: clearAttachments } = useAttachments()
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
const { openTurnId } = useTurns()

const sending = ref(false)
const composerRef = ref<InstanceType<typeof ChatComposer>>()

const activeProfile = computed(() => profiles.value.find((p) => p.id === selectedProfile.value))

// Block grouping is driven by ACP turn ids (Rust mints one per
// `session/prompt` and stamps every notification it emits with that
// id; see `acp:turn-started` / `acp:turn-ended`). Assistant entries
// carrying the same `turnId` collapse into one block; user turns —
// which arrive before any `TurnStarted` for the reply — sit in their
// own per-message block. Anchored render order within an assistant
// block stays: thoughts + plans → tool-call grid → assistant reply
// body.
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
  /// Group key — ACP turn id for assistant blocks, synthetic per-row
  /// key for user blocks (each user message stays in its own block).
  groupKey: string
  /// Set when the block represents a real ACP turn; `undefined` for
  /// user blocks and for spontaneous out-of-turn agent updates.
  turnId?: string
  startedAt: number
  streamEntries: TimelineStream[]
  toolCalls: TimelineTool[]
  turnEntries: TimelineTurn[]
}

function entryRole(entry: TimelineEntry): Role {
  if (entry.kind === 'turn') {
    return entry.turn.role === TurnRole.User ? Role.User : Role.Assistant
  }

  return Role.Assistant
}

function entryTurnId(entry: TimelineEntry): string | undefined {
  if (entry.kind === 'turn') {
    return entry.turn.turnId
  }
  if (entry.kind === 'stream') {
    return entry.item.turnId
  }

  return entry.call.turnId
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
    const role = entryRole(entry)
    const turnId = entryTurnId(entry)
    // User turns and unanchored assistant entries each get their own
    // block (synthetic key); turn-anchored assistant entries fold into
    // a shared block keyed by the turn id.
    const groupKey =
      role === Role.Assistant && turnId !== undefined ? `turn:${turnId}` : `solo:${role}:${entry.createdAt}:${entry.kind}`
    const last = blocks[blocks.length - 1]
    let block: TimelineBlock
    if (last && last.groupKey === groupKey) {
      block = last
    } else {
      block = {
        role,
        groupKey,
        turnId: role === Role.Assistant ? turnId : undefined,
        startedAt: entry.createdAt,
        streamEntries: [],
        toolCalls: [],
        turnEntries: []
      }
      blocks.push(block)
    }
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

// The "live" block is the assistant block whose ACP turn id is still
// open (`acp:turn-ended` hasn't landed for it). Once the matching
// TurnEnded arrives, `openTurnId` clears and the pulse stops.
const liveBlockIdx = computed<number>(() => {
  const open = openTurnId.value
  if (!open) {
    return -1
  }

  return timelineBlocks.value.findIndex((b) => b.turnId === open)
})

let stopStream: (() => void) | undefined

function firePermission(action: 'allow' | 'deny'): void {
  if (isEditableTarget(document.activeElement)) {
    return
  }
  // TODO(K-281 follow-up): Tab = next row cycling. Today the approval
  // keybind always addresses the oldest-active (first non-queued) prompt.
  const active = permissionPrompts.value.find((p) => !p.queued) ?? permissionPrompts.value[0]
  if (!active) {
    return
  }
  log.info('keybind invoked', { action, target: 'permission' })
  if (action === 'allow') {
    void onAllow(active.requestId)
  } else {
    void onDeny(active.requestId)
  }
}

const { keymaps } = useKeymaps()
const { closeAll: closeAllPalettes } = usePalette()

useKeymap(
  () => document,
  (): KeymapEntry[] => {
    if (!keymaps.value) {
      return []
    }

    return [
      {
        binding: keymaps.value.approvals.allow,
        handler: () => {
          firePermission('allow')
        }
      },
      {
        binding: keymaps.value.approvals.deny,
        handler: () => {
          firePermission('deny')
        }
      },
      {
        binding: keymaps.value.palette.open,
        handler: () => {
          openRootPalette()

          return true
        }
      },
      {
        binding: keymaps.value.palette.close,
        handler: () => {
          closeAllPalettes()
        }
      },
      // TODO: replace with keymaps.value.palette.skills.open once the
      // Rust-side [keymaps.palette.skills] group lands in its own issue.
      {
        binding: { modifiers: [Modifier.Ctrl], key: 'space' },
        handler: () => {
          openSkillsPalette()

          return true
        }
      }
    ]
  }
)

onMounted(async () => {
  try {
    stopStream = await startSessionStream()
  } catch (err) {
    log.error('invoke failed', { command: 'startSessionStream' }, err)
    pushToast(ToastTone.Err, `stream bind failed: ${String(err)}`)
  }
})

onUnmounted(() => {
  stopStream?.()
  stopStream = undefined
})

// Skill attachments are per-turn but tied to the active instance —
// switching to another instance mid-compose discards any pending
// picks (they were assembled against the previous instance's context).
watch(activeInstanceId, (next, prev) => {
  if (prev && next !== prev) {
    clearAttachments()
  }
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

// Mint a fresh instance UUID the backend will adopt. Matches
// `AcpInstances::InstanceKey` — a v4 UUID, opaque to the UI except
// for identity. Used when there's no active instance yet (first
// submit, or after a "new session" reset).
function mintInstanceId(): InstanceId {
  return crypto.randomUUID()
}

function onSubmit(payload: { text: string; attachments: unknown[] }): void {
  const { text, attachments } = payload
  // Skill / resource attachments live in the `useAttachments` singleton
  // (K-268 palette pushes onto it). They snapshot at submit time so a
  // resubmit after cancel sends the same set; submit-ack clears.
  const skillAttachments = [...pendingAttachments.value]
  log.info('composer submit', {
    text_len: text.length,
    image_attachments: attachments.length,
    skill_attachments: skillAttachments.length,
    profileId: selectedProfile.value
  })
  sending.value = true

  // Pick the instance UUID up-front so we can push the user turn
  // BEFORE `submit` resolves. Without a known id, we'd have to push
  // in `.then()` — but by then the agent has already streamed back
  // session/update events that landed with lower per-instance seq
  // than the user turn, and those events absorb into the prior
  // turn's assistant block ("thought blocks go to prior turn" bug).
  // Client-generated UUIDs let the backend adopt the id on first
  // sight, closing the race.
  const instanceId = activeInstanceId.value ?? mintInstanceId()
  useActiveInstance().set(instanceId)
  pushTranscriptChunk(instanceId, '', {
    sessionUpdate: 'user_message_chunk',
    content: { type: 'text', text }
  })

  submit({ text, instanceId, profileId: selectedProfile.value, attachments: skillAttachments })
    .then(() => {
      composerRef.value?.clear()
      clearAttachments()
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
      <ChatTurn
        v-for="(block, blockIdx) in timelineBlocks"
        :key="block.groupKey"
        :role="block.role"
        :live="blockIdx === liveBlockIdx"
      >
        <template v-for="entry in block.streamEntries" :key="`stream-${entry.createdAt}`">
          <ChatStreamCard
            v-if="entry.item.kind === StreamItemKind.Thought"
            :kind="StreamKind.Thinking"
            :active="blockIdx === liveBlockIdx"
            label="thought"
            >{{ entry.item.text }}</ChatStreamCard
          >
          <ChatStreamCard
            v-else-if="entry.item.kind === StreamItemKind.Plan"
            :kind="StreamKind.Planning"
            :active="blockIdx === liveBlockIdx"
            label="plan"
            :items="mapPlanItems(entry.item.entries)"
          />
        </template>

        <!-- provider passed `undefined` for now: resolves to baseRegistry. Plumb -->
        <!-- `activeProfile?.agent` → `profiles_list`'s vendor once per-adapter overrides land. -->
        <ChatToolChips v-if="block.toolCalls.length > 0" :items="block.toolCalls.map((t) => formatToolCall(t.call))" grouped />

        <template v-for="entry in block.turnEntries" :key="`turn-${entry.createdAt}`">
          <ChatBody :role="entry.turn.role === TurnRole.User ? Role.User : Role.Assistant">{{ entry.turn.text }}</ChatBody>
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

  <CommandPalette />
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
