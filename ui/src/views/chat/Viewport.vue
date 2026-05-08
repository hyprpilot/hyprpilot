<script setup lang="ts">
/**
 * Chat transcript viewport. Reads off `useChatViewport` (which wraps
 * `useInstanceChatInfiniteQuery` + live-event patches + page-trim
 * policy) and feeds the resulting blocks into the virtualized scroll
 * surface.
 *
 * **Variable-height virtualization** via `@tanstack/vue-virtual`:
 *
 * - `useVirtualizer` keyed by `block.groupKey` (stable across renders;
 *   array-index keys would invalidate measurements on every prepend).
 * - Each rendered row carries `data-index` + a `:ref` callback into
 *   `virtualizer.measureElement`. Without BOTH bindings, the virtualizer
 *   can't attach its `ResizeObserver` and every row stays at the
 *   `estimateSize` value forever — that produces visible gaps between
 *   short rows and overlap on tall ones.
 * - `shouldAdjustScrollPositionOnItemSizeChange` returns `true` when the
 *   resized item is above the current viewport: when an older page
 *   measures in (taller than the estimate), the scroll offset
 *   compensates by the delta so the captain's currently-visible content
 *   stays anchored. This is the maintainer-recommended fix for the
 *   prepend-page jump (TanStack Virtual discussion #1013).
 * - Streaming text into an existing row: the per-row `ResizeObserver`
 *   fires automatically — no manual `measure()` call needed.
 *
 * **Backward pagination**: a `@scroll` handler watches `scrollTop` and
 * triggers `viewport.fetchNextPage()` when the captain crosses
 * `LOAD_MORE_THRESHOLD_PX` from the top. The previous DOM-sentinel +
 * `useIntersectionObserver` was broken because the observer's default
 * `root` is the document, not the chat scroll container — the sentinel
 * never intersected when the captain scrolled the chat up. A direct
 * scrollTop check sidesteps that entirely.
 *
 * **Stick-to-bottom**: `useStickToBottom` provides the `stuck` boolean
 * (the captain is at the tail). When new blocks land while stuck, we
 * call `virtualizer.scrollToIndex(last, { align: 'end' })` in
 * `nextTick` so the live event flows down naturally. This works
 * alongside the observer-driven scroll the composable does internally:
 * the virtualizer call gives us index-aware end-alignment that handles
 * the "last row still growing post-scroll" case during streaming.
 */
import { useVirtualizer } from '@tanstack/vue-virtual'
import { useNow } from '@vueuse/core'
import { computed, nextTick, ref, watch } from 'vue'

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
  useSessionInfo,
  useSnapshotHydration,
  useStickToBottom,
  useTurns,
  type PlanEntry,
  type WireToolCall,
  type InstanceId
} from '@composables'
import { format, formatDuration } from '@lib'

/// Distance from the top (in px) at which we trigger backward
/// pagination. Generous enough that `fetchNextPage()` resolves before
/// the captain runs out of content; tight enough not to fire while
/// the captain is reading mid-list.
const LOAD_MORE_THRESHOLD_PX = 240

/// Estimated row height for unmeasured blocks. Bias toward the
/// **median** actual height so over-estimates and under-estimates
/// roughly cancel — that minimises offset jitter as rows measure in.
/// Real chat blocks vary from ~32px (one-line user prompt) to 1500px+
/// (long agent reply with tool chips); 200 is the rough median across
/// the development sessions.
const ESTIMATE_SIZE_PX = 200

const props = defineProps<{
  /// Captain's "session is restoring" gate — keeps the scoped
  /// `<Loading>` overlay painted on top while transcript replay is
  /// in flight.
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
const { info: sessionInfo } = useSessionInfo()
const { openTurnId, turns: turnRecords } = useTurns()

// Hydrate `useTurns` from the snapshot meta query. Live events that
// streamed before this component mounted (focus-switch, remote bridge
// authenticated mid-session) are invisible to the live router; the
// daemon mirror has them and ships them on `MetaSnapshot.turns`.
useSnapshotHydration(instanceId)

// Resolve adapter for the active instance's agent. Snapshot tool-call
// entries don't carry agentId on the wire; we look it up off the
// active session's meta so the formatter can produce icons +
// state-aware stats.
const adapterForActive = computed(() => {
  const id = sessionInfo.value.agent

  return id ? adapterFor(id) : undefined
})

const scrollEl = ref<HTMLElement>()

const { stuck } = useStickToBottom(scrollEl)

watch(stuck, (next) => {
  viewport.onStuckChange(next)
})

// ── Virtualization ──────────────────────────────────────────────────
const virtualizer = useVirtualizer(
  computed(() => ({
    count: blocks.value.length,
    getScrollElement: () => scrollEl.value ?? null,
    estimateSize: () => ESTIMATE_SIZE_PX,
    overscan: 8,
    getItemKey: (i: number) => blocks.value[i]?.groupKey ?? i,
    /**
     * Compensate scroll offset when an off-screen-above row remeasures.
     * Stops backward-page prepends from yanking the captain's content
     * out of view (TanStack Virtual discussion #1013, default in
     * forthcoming versions).
     */
    shouldAdjustScrollPositionOnItemSizeChange: (item: { start: number }, _delta: number, instance: { scrollOffset: number | null }) => {
      const offset = instance.scrollOffset ?? 0

      return item.start < offset
    }
  }))
)

const virtualRows = computed(() => virtualizer.value.getVirtualItems())
const totalSize = computed(() => virtualizer.value.getTotalSize())

// Follow the tail when stuck and new blocks land. nextTick lets the
// virtualizer process the count change before we ask it to scroll.
watch(
  () => blocks.value.length,
  async(next) => {
    if (!stuck.value || next === 0) {
      return
    }
    await nextTick()
    virtualizer.value.scrollToIndex(next - 1, { align: 'end' })
  }
)

// ── Backward pagination ─────────────────────────────────────────────
function onScroll(): void {
  const el = scrollEl.value

  if (!el) {
    return
  }

  if (!viewport.hasNextPage.value) {
    return
  }

  if (viewport.isFetchingNextPage.value) {
    return
  }

  if (el.scrollTop < LOAD_MORE_THRESHOLD_PX) {
    void viewport.fetchNextPage()
  }
}

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
  <div ref="scrollEl" class="chat-transcript" data-testid="chat-transcript" :data-instance-id="instanceId ?? ''" @scroll="onScroll">
    <Loading v-if="props.restoring" mode="scoped" status="restoring session — replaying transcript" />

    <slot v-if="isEmpty" name="empty" />

    <template v-else>
      <!-- Loading chip pinned at the top while a backward page is in
           flight. Sits outside the virtualized spacer so its height
           doesn't compete with row offsets. -->
      <div v-if="viewport.isFetchingNextPage.value" class="chat-load-chip animate-pulse" data-testid="chat-load-chip">loading earlier…</div>

      <!-- Virtualized spacer: total height matches the sum of
           measured/estimated row sizes. Rows position themselves via
           `transform: translateY(...)` inside this relative parent. -->
      <div class="chat-virtual-host" :style="{ height: `${totalSize}px` }">
        <div
          v-for="row in virtualRows"
          :key="row.key"
          :data-index="row.index"
          :ref="(el) => virtualizer.measureElement(el as Element | null)"
          class="chat-virtual-row"
          :style="{ transform: `translateY(${row.start}px)` }"
        >
          <Turn
            v-if="blocks[row.index]"
            :role="blocks[row.index]!.role"
            :live="row.index === liveBlockIdx"
            :elapsed="elapsedFor(blocks[row.index]!.turnId)"
            :usage="usageFor(blocks[row.index]!.turnId)"
          >
            <StreamCard
              v-if="combinedThoughtText(blocks[row.index]!).length > 0 || hasThinkingSignal(blocks[row.index]!)"
              :kind="StreamKind.Thinking"
              :active="row.index === liveBlockIdx"
              label="thought"
              :elapsed="thinkingElapsedFor(blocks[row.index]!)"
              :text="combinedThoughtText(blocks[row.index]!).length > 0 ? combinedThoughtText(blocks[row.index]!) : undefined"
            />
            <template v-for="entry in blocks[row.index]!.streamEntries" :key="`stream-${entry.createdAt}`">
              <StreamCard
                v-if="entry.item.kind === StreamItemKind.Plan"
                :kind="StreamKind.Planning"
                :active="row.index === liveBlockIdx"
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
              <ChangeBanner
                v-else-if="entry.item.kind === StreamItemKind.SystemPromptInjected"
                kind="system prompt"
                :to="systemPromptLabel(entry.item.files)"
              />
            </template>

            <ToolChips v-if="blocks[row.index]!.toolCalls.length > 0" :views="blocks[row.index]!.toolCalls.map((t) => format(t.call, adapterForActive))" />

            <template v-for="entry in blocks[row.index]!.toolCalls" :key="`term-${entry.call.toolCallId}`">
              <TerminalCard v-if="terminalIdForCall(entry.call)" :terminal-id="terminalIdForCall(entry.call) ?? ''" :instance-id="instanceId" @cancel="emit('cancel')" />
            </template>

            <template v-for="entry in blocks[row.index]!.turnEntries" :key="`turn-${entry.createdAt}`">
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
</template>

<style scoped>
@reference '../../assets/styles.css';

.chat-transcript {
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
  position: relative;
  /* Stop the browser's native scroll-anchoring from fighting the
   * virtualizer's own scroll-offset adjustment when rows above remeasure. */
  overflow-anchor: none;
}

.chat-virtual-host {
  /* Inner spacer — rows are absolutely positioned within. The host's
   * height is set inline from the virtualizer's `getTotalSize()`. */
  position: relative;
  width: 100%;
}

.chat-virtual-row {
  /* Absolute children of the relative host. Padding lives on the row
   * itself because absolute children's `width: 100%` resolves against
   * the parent's padding-box width — padding on the parent has no
   * effect on them. `box-sizing: border-box` keeps the width math
   * intact so 100% means "full host width including padding". */
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  box-sizing: border-box;
  padding: 0 0.875rem 0 0.25rem;
}

.chat-load-chip {
  @apply rounded text-[0.7rem];
  margin: 0.5rem auto 0.25rem;
  padding: 0.125rem 0.5rem;
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg-dim);
  border: 1px solid var(--theme-border-soft);
  align-self: center;
}
</style>
