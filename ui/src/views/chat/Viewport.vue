<script setup lang="ts">
/**
 * Chat transcript viewport (Phase C1).
 *
 * Replaces the in-line `<Turn v-for>` block formerly rendered inside
 * `Overlay.vue::.chat-transcript`. Reads off `useChatViewport` (which
 * wraps `useInstanceChatInfiniteQuery` + live-event patches + page-
 * trim policy) and feeds the resulting `SeqTranscriptItem[]` into
 * the same `<Turn>` / `<StreamCard>` / `<ToolChips>` /
 * `<TerminalCard>` / `<ChatBody>` leaf components Overlay.vue used
 * before. Layout invariants are unchanged — `.chat-transcript-inner`
 * still owns the gutter padding so `<Loading>` covers paint
 * edge-to-edge inside the wrapper.
 *
 * Virtualization: TanStack `useVirtualizer` with `count = blocks.length`,
 * `estimateSize` set to a conservative average (160px), `overscan: 5`.
 * Each virtualized row is one `<Turn>` block — heavy bodies (markdown
 * HTML, tool diffs, terminal scrollback) unmount when their row leaves
 * the viewport, reclaiming memory automatically.
 *
 * Backward pagination: a sentinel element renders above the first
 * virtual row whenever `hasNextPage` is true. `useIntersectionObserver`
 * fires `fetchNextPage()` when the sentinel enters the viewport. While
 * a fetch is in flight, the sentinel hosts a small `animate-pulse`
 * loading chip. Once the daemon reports `hasMore: false`, the
 * sentinel + chip both hide.
 *
 * Live behaviour: live `acp:transcript` events land on the latest
 * page via `setQueryData` (in `useChatViewport`). The body's
 * `<ChatBody markdown>` re-renders the affected row only; older
 * rows stay stable.
 */
import { useVirtualizer } from '@tanstack/vue-virtual'
import { useIntersectionObserver, useNow } from '@vueuse/core'
import { computed, ref, watch } from 'vue'

import Attachments from './Attachments.vue'
import Body from './Body.vue'
import ChangeBanner from './ChangeBanner.vue'
import StreamCard from './StreamCard.vue'
import TerminalCard from './TerminalCard.vue'
import ToolChips from './ToolChips.vue'
import Turn from './Turn.vue'
import { Loading, Role, StreamKind, PlanStatus, type PlanItem } from '@components'
import {
  StreamItemKind,
  TurnRole,
  timelineBlocksFromSnapshot,
  useActiveInstance,
  useAgentRegistry,
  useChatViewport,
  useStickToBottom,
  useTurns,
  type PlanEntry,
  type WireToolCall,
  type InstanceId
} from '@composables'
import { format, formatDuration } from '@lib'

const props = defineProps<{
  /// Captain's "session is restoring" gate — keeps the scoped
  /// `<Loading>` overlay painted on top while transcript replay is
  /// in flight. Falls through unchanged from Overlay.vue.
  restoring?: boolean
}>()

const emit = defineEmits<{
  /// Fires when the captain hits the per-tool-call cancel button on
  /// a live terminal — Overlay.vue routes this to `session_cancel`.
  cancel: []
  /// Fires when an attachment pill on a captain turn is clicked.
  'attachment-open': [attachment: import('@ipc').Attachment]
}>()

const { id: activeInstanceId } = useActiveInstance()
const instanceId = computed<InstanceId | undefined>(() => activeInstanceId.value)
const viewport = useChatViewport(instanceId)
const blocks = computed(() => timelineBlocksFromSnapshot(viewport.items.value, instanceId.value ?? 'snapshot'))

const { adapterFor } = useAgentRegistry()
const { openTurnId, turns: turnRecords } = useTurns()

// Scroll element + sentinel refs. The viewport's outer scroll
// container is the rem-padded wrapper; the inner positioned div
// hosts the virtualized rows.
const scrollEl = ref<HTMLElement>()
const sentinelEl = ref<HTMLElement>()

const { stuck } = useStickToBottom(scrollEl)

// Drive page-trim off the stuck signal: only fires the trim when
// the captain returns to the bottom AND the cache has more pages
// than `MAX_PAGES_KEPT`. Throttling lives inside the composable.
watch(stuck, (next) => {
  viewport.onStuckChange(next)
})

// Virtualizer instance. `getScrollElement` returns the outer scroll
// container; `count` follows blocks.length reactively (the helper
// re-reads whenever its dependencies change).
const virtualizer = useVirtualizer(
  computed(() => ({
    count: blocks.value.length,
    getScrollElement: () => scrollEl.value ?? null,
    estimateSize: () => 160,
    overscan: 5
  }))
)

const virtualItems = computed(() => virtualizer.value.getVirtualItems())
const totalSize = computed(() => virtualizer.value.getTotalSize())

// Backward-pagination sentinel: triggers `fetchNextPage` when the
// captain scrolls to the very top. Re-armed each time the
// observer's intersecting state flips false → true.
useIntersectionObserver(sentinelEl, (entries) => {
  for (const entry of entries) {
    if (!entry.isIntersecting) {
      continue
    }

    if (!viewport.hasNextPage.value) {
      continue
    }

    if (viewport.isFetchingNextPage.value) {
      continue
    }
    void viewport.fetchNextPage()
  }
})

const liveBlockIdx = computed<number>(() => {
  const open = openTurnId.value

  if (!open) {
    return -1
  }

  return blocks.value.findIndex((b) => b.turnId === open)
})

const liveNow = useNow({ interval: 1000 })

function liveNowMs(): number {
  return liveNow.value.getTime()
}

const turnDurationLabels = computed<Map<string, string>>(() => {
  const out = new Map<string, string>()
  const now = liveNowMs()

  for (const t of turnRecords.value) {
    if (typeof t.startedAtMs !== 'number' || t.startedAtMs === 0) {
      continue
    }
    const end = typeof t.endedAtMs === 'number' ? t.endedAtMs : now
    const elapsed = Math.max(0, end - t.startedAtMs)

    if (!Number.isFinite(elapsed)) {
      continue
    }
    out.set(t.id, formatDuration(elapsed))
  }

  return out
})

function elapsedFor(turnId?: string): string | undefined {
  if (!turnId) {
    return undefined
  }

  return turnDurationLabels.value.get(turnId)
}

function usageFor(turnId?: string) {
  if (!turnId) {
    return undefined
  }

  return turnRecords.value.find((rec) => rec.id === turnId)?.usage
}

interface ThinkingElapsedBlock {
  turnId?: string
  thoughts: { call: { startedAtMs: number; completedAtMs?: number } }[]
}

function thinkingElapsedFor(block: ThinkingElapsedBlock): string | undefined {
  const now = liveNowMs()
  let totalMs = 0
  let hasSignal = false

  if (block.turnId !== undefined) {
    const turn = turnRecords.value.find((rec) => rec.id === block.turnId)

    if (turn !== undefined) {
      const closed = typeof turn.thinkingMs === 'number' ? turn.thinkingMs : 0
      const open = typeof turn.thinkingOpenAtMs === 'number' ? Math.max(0, now - turn.thinkingOpenAtMs) : 0
      const stream = closed + open

      if (stream > 0 || turn.thinkingOpenAtMs !== undefined) {
        totalMs += stream
        hasSignal = true
      }
    }
  }

  for (const entry of block.thoughts) {
    const s = entry.call.startedAtMs

    if (typeof s !== 'number' || s <= 0) {
      continue
    }
    const c = entry.call.completedAtMs
    const end = typeof c === 'number' ? c : now

    totalMs += Math.max(0, end - s)
    hasSignal = true
  }

  if (!hasSignal || !Number.isFinite(totalMs)) {
    return undefined
  }

  return formatDuration(totalMs)
}

function hasThinkingSignal(block: { turnId?: string; thoughts: { call: { startedAtMs: number } }[] }): boolean {
  if (block.turnId !== undefined) {
    const turn = turnRecords.value.find((rec) => rec.id === block.turnId)

    if (turn !== undefined) {
      const closed = typeof turn.thinkingMs === 'number' ? turn.thinkingMs : 0

      if (closed > 0 || turn.thinkingOpenAtMs !== undefined) {
        return true
      }
    }
  }

  for (const entry of block.thoughts) {
    if (typeof entry.call.startedAtMs === 'number' && entry.call.startedAtMs > 0) {
      return true
    }
  }

  return false
}

function thoughtText(call: { title?: string; content: { type?: string; text?: string }[]; rawInput?: Record<string, unknown> }): string {
  const parts: string[] = []
  const summary = call.title?.trim()

  if (summary && summary.length > 0) {
    parts.push(`**${summary}**`)
  }

  for (const c of call.content ?? []) {
    if (typeof c.text === 'string' && c.text.trim().length > 0) {
      parts.push(c.text)
    }
  }

  if (parts.length === 0 && call.rawInput) {
    const raw = call.rawInput.thought ?? call.rawInput.text ?? call.rawInput.description

    if (typeof raw === 'string') {
      parts.push(raw)
    }
  }

  return parts.join('\n\n')
}

function combinedThoughtText(block: {
  thoughts: { createdAt: number; call: WireToolCall }[]
  streamEntries: { createdAt: number; item: { kind: StreamItemKind; text?: string } }[]
}): string {
  const merged: { createdAt: number; text: string }[] = []

  for (const entry of block.thoughts) {
    const text = thoughtText(entry.call)

    if (text.length > 0) {
      merged.push({ createdAt: entry.createdAt, text })
    }
  }

  for (const entry of block.streamEntries) {
    if (entry.item.kind !== StreamItemKind.Thought) {
      continue
    }
    const text = entry.item.text ?? ''

    if (text.length > 0) {
      merged.push({ createdAt: entry.createdAt, text })
    }
  }
  merged.sort((a, b) => a.createdAt - b.createdAt)

  return merged.map((m) => m.text).join('\n\n')
}

function systemPromptLabel(files: readonly string[]): string {
  if (files.length === 0) {
    return 'attached'
  }
  const baseNames = files.map((f) => f.split('/').pop() ?? f)

  if (baseNames.length <= 3) {
    return baseNames.join(', ')
  }

  return `${baseNames.slice(0, 3).join(', ')} +${baseNames.length - 3} more`
}

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

function terminalIdForCall(call: { rawInput?: Record<string, unknown> }): string | undefined {
  const raw = call.rawInput

  if (!raw) {
    return undefined
  }
  const candidate = raw.terminal_id ?? raw.terminalId

  return typeof candidate === 'string' && candidate.length > 0 ? candidate : undefined
}

const isEmpty = computed(() => blocks.value.length === 0)

defineExpose({ scrollEl })
</script>

<template>
  <div ref="scrollEl" class="chat-transcript" data-testid="chat-transcript" :data-instance-id="instanceId ?? ''">
    <Loading v-if="props.restoring" mode="scoped" status="restoring session — replaying transcript" />

    <div class="chat-transcript-inner">
      <slot v-if="isEmpty" name="empty" />
      <template v-else>
        <!-- Sentinel + loading chip — captain's pull-up affordance.
             The sentinel is always rendered above the first virtual
             row whenever `hasNextPage` is true; the IntersectionObserver
             attached above triggers `fetchNextPage` when the
             sentinel enters the viewport. The chip animates only
             while the fetch is in flight. -->
        <div v-if="viewport.hasNextPage.value" ref="sentinelEl" class="chat-load-sentinel" data-testid="chat-load-sentinel">
          <div v-if="viewport.isFetchingNextPage.value" class="chat-load-chip animate-pulse" data-testid="chat-load-chip">loading earlier…</div>
        </div>

        <!-- Virtualized list. The outer div is the absolute-positioning
             host; each virtual row sits at `transform: translateY(...)`
             with `position: absolute; top: 0; left: 0; width: 100%`. -->
        <div class="chat-virtual-host" :style="{ height: `${totalSize}px` }">
          <div
            v-for="virtual in virtualItems"
            :key="blocks[virtual.index]?.groupKey ?? virtual.key"
            class="chat-virtual-row"
            :style="{ transform: `translateY(${virtual.start}px)` }"
            :data-index="virtual.index"
          >
            <Turn
              v-if="blocks[virtual.index]"
              :role="blocks[virtual.index]!.role"
              :live="virtual.index === liveBlockIdx"
              :elapsed="elapsedFor(blocks[virtual.index]!.turnId)"
              :usage="usageFor(blocks[virtual.index]!.turnId)"
            >
              <StreamCard
                v-if="combinedThoughtText(blocks[virtual.index]!).length > 0 || hasThinkingSignal(blocks[virtual.index]!)"
                :kind="StreamKind.Thinking"
                :active="virtual.index === liveBlockIdx"
                label="thought"
                :elapsed="thinkingElapsedFor(blocks[virtual.index]!)"
                :text="combinedThoughtText(blocks[virtual.index]!).length > 0 ? combinedThoughtText(blocks[virtual.index]!) : undefined"
              />
              <template v-for="entry in blocks[virtual.index]!.streamEntries" :key="`stream-${entry.createdAt}`">
                <StreamCard
                  v-if="entry.item.kind === StreamItemKind.Plan"
                  :kind="StreamKind.Planning"
                  :active="virtual.index === liveBlockIdx"
                  label="plan"
                  :items="mapPlanItems(entry.item.entries)"
                />
                <ChangeBanner
                  v-else-if="entry.item.kind === StreamItemKind.ModeChange"
                  kind="mode"
                  :to="entry.item.name ?? entry.item.modeId"
                  :from="entry.item.prevName ?? entry.item.prevModeId"
                />
                <ChangeBanner
                  v-else-if="entry.item.kind === StreamItemKind.ModelChange"
                  kind="model"
                  :to="entry.item.name ?? entry.item.modelId"
                  :from="entry.item.prevName ?? entry.item.prevModelId"
                />
                <ChangeBanner
                  v-else-if="entry.item.kind === StreamItemKind.ConfigOptionChange"
                  :kind="entry.item.categoryId"
                  :to="entry.item.name ?? entry.item.value"
                  :from="entry.item.prevName ?? entry.item.prevValue"
                />
                <ChangeBanner v-else-if="entry.item.kind === StreamItemKind.SystemPromptInjected" kind="system prompt" :to="systemPromptLabel(entry.item.files)" />
              </template>

              <ToolChips v-if="blocks[virtual.index]!.toolCalls.length > 0" :views="blocks[virtual.index]!.toolCalls.map((t) => format(t.call, adapterFor(t.call.agentId)))" />

              <template v-for="entry in blocks[virtual.index]!.toolCalls" :key="`term-${entry.call.toolCallId}`">
                <TerminalCard v-if="terminalIdForCall(entry.call)" :terminal-id="terminalIdForCall(entry.call) ?? ''" :instance-id="instanceId" @cancel="emit('cancel')" />
              </template>

              <template v-for="entry in blocks[virtual.index]!.turnEntries" :key="`turn-${entry.createdAt}`">
                <Body v-if="entry.turn.role === TurnRole.Agent" :role="Role.Assistant" :text="entry.turn.text" markdown />
                <template v-else>
                  <Body :role="Role.User" :text="entry.turn.text" markdown />
                  <Attachments
                    v-if="entry.turn.attachments && entry.turn.attachments.length > 0"
                    :attachments="entry.turn.attachments"
                    @open="(att) => emit('attachment-open', att)"
                  />
                </template>
              </template>
            </Turn>
          </div>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.chat-transcript {
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
  position: relative;
}

.chat-transcript-inner {
  @apply flex min-h-0 flex-1 flex-col;
  padding: 0 0.875rem 0 0.25rem;
}

.chat-load-sentinel {
  @apply flex w-full justify-center py-2;
  min-height: 1.5rem;
}

.chat-load-chip {
  @apply rounded text-[0.7rem];
  padding: 0.125rem 0.5rem;
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg-dim);
  border: 1px solid var(--theme-border-soft);
}

.chat-virtual-host {
  position: relative;
  width: 100%;
}

.chat-virtual-row {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
}
</style>
