<script setup lang="ts">
/**
 * Chat transcript viewport. Reads off `useChatViewport` (which wraps
 * `useInstanceChatInfiniteQuery` + live-event patches + page-trim
 * policy) and feeds the resulting blocks into a plain `v-for` render.
 *
 * **No virtualization.** TanStack Vue Virtual was tried twice and
 * pulled out twice. Variable-height content + streaming chunks
 * creates a tight ResizeObserver / `triggerRef` / re-measure cycle
 * that never converges:
 *
 *   1. virtualizer.measureElement(row) → onChange → triggerRef(state)
 *   2. virtualRows recomputes (positions shift)
 *   3. Vue re-renders the row, content unchanged but ref re-fires
 *   4. ResizeObserver fires for the changed-size row (head row keeps
 *      growing during streaming)
 *   5. Goto 1
 *
 * Vue caps the loop at 100 iterations and throws "Maximum recursive
 * updates exceeded in component <Viewport>". The non-virtualized
 * `v-for` is stable: page-trim already bounds the live DOM to ~150
 * rows (`useChatViewport.MAX_PAGES_KEPT` × `DEFAULT_CHAT_LIMIT`),
 * which Vue handles without breaking a sweat. Re-virtualize later
 * if a real memory ceiling shows up; the bottleneck today is
 * correctness.
 *
 * **Backward pagination**: a `@scroll` handler watches `scrollTop` and
 * triggers `viewport.fetchNextPage()` when the captain crosses
 * `LOAD_MORE_THRESHOLD_PX` from the top.
 *
 * **Stick-to-bottom**: `useStickToBottom` already runs a
 * MutationObserver + ResizeObserver pair on the scroll container and
 * scrolls to the tail on every mutation while `stuck` is true. No
 * extra Vue watcher needed.
 */
import { faChevronDown } from '@fortawesome/free-solid-svg-icons'
import { useEventListener, useNow } from '@vueuse/core'
import { computed, nextTick, ref } from 'vue'

import Attachments from './Attachments.vue'
import Body from './Body.vue'
import ChangeBanner from './ChangeBanner.vue'
import StreamCard from './StreamCard.vue'
import TerminalCard from './TerminalCard.vue'
import ToolChips from './ToolChips.vue'
import Turn from './Turn.vue'
import { Loading, Role, StreamKind, PlanStatus, type PlanItem } from '@components'
import {
  isEditableTarget,
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

// `scrollEl` declared up-front so `useChatViewport` can derive its
// fetch page size from `clientHeight`. The ref is undefined until
// mount; `viewportPageSize`'s fallback returns the minimum size so
// the initial fetch races a sane lower bound, and every backward
// page picks up the real viewport extent.
const scrollEl = ref<HTMLElement>()
/// Flipped on the first captain-initiated scroll event after mount.
/// Both `fetchNextPage` (load older) and `evictExtraPages` (drop
/// trailing pages) gate on this so they don't fire during the
/// initial paint of a freshly-mounted viewport. Without the gate:
/// `useStickToBottom.onMounted` writes `scrollTop = scrollHeight`,
/// the browser fires a synthetic `scroll` event for that
/// assignment, the handler sees `scrollTop < LOAD_MORE_THRESHOLD_PX`
/// while the page is mid-render (the new instance's head page may
/// only have 1-2 items rendered so far → `scrollHeight ≈
/// clientHeight` → `scrollTop ≈ 0`) and burns a backward fetch on
/// content the captain hasn't even scrolled past yet. This was the
/// captain's "loading earlier history after switching" bug.
///
/// Resets to `false` on every mount (the `:key="activeInstanceId"`
/// on `<ChatViewport>` in `Overlay.vue` forces a clean remount per
/// instance flip, so this ref is freshly `false` for each instance).
const hasUserScrolled = ref(false)

const viewport = useChatViewport(instanceId, { scrollEl })
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

const { stuck, scrollToBottom } = useStickToBottom(scrollEl)

// `stuck` is the auto-scroll signal — strict 64px-from-bottom
// threshold so a captain reading 1 viewport above the foot
// doesn't get yanked back on every chunk. We do NOT use it for
// eviction; eviction fires from `onScroll` whenever the captain
// is within ~one viewport of the bottom, which is wider than the
// auto-scroll window so cache cleanup is prompt without
// disturbing the read-history flow.

/// Floating chevron click — drop extra pages, await Vue's DOM
/// patch, THEN jump to the foot. Eviction shrinks `data.pages`
/// from the OLDEST entry, which renders at the TOP of the DOM
/// (`use-chat-viewport.items` walks pages last→first).
/// `evictExtraPages()` calls `setQueryData` synchronously, but
/// Vue's reactive DOM patch flushes on the next microtask — so
/// without `await nextTick()`, `scrollToBottom()` would read the
/// PRE-eviction `scrollHeight`, scroll past the new tail, and the
/// browser would clamp scrollTop in a visible second step.
/// Awaiting nextTick lets Vue flush the eviction patch first;
/// `scrollToBottom()` then lands exactly at the new bottom in one
/// motion.
async function goToBottom(): Promise<void> {
  viewport.evictExtraPages()
  await nextTick()
  scrollToBottom()
}

// ── Rendering ──────────────────────────────────────────────────────
//
// **No virtualization.** TanStack Vue Virtual was tried twice and
// pulled out twice for the same root cause: variable-height content
// + streaming chunks creates a tight ResizeObserver / `triggerRef` /
// re-measure cycle that never converges:
//
//   1. virtualizer.measureElement(row) → onChange → triggerRef(state)
//   2. virtualRows recomputes (positions shift)
//   3. Vue re-renders the row, content unchanged but ref re-fires
//   4. ResizeObserver fires for the changed-size row (head row keeps
//      growing during streaming)
//   5. Goto 1
//
// Vue caps the loop at 100 iterations and throws "Maximum recursive
// updates exceeded in component <Viewport>". Under streaming reply
// or session/load replay it fires every chunk.
//
// The non-virtualized `v-for` over `blocks` is stable: page-trim
// already bounds the live DOM to ~150 rows
// (`useChatViewport.MAX_PAGES_KEPT` × `DEFAULT_CHAT_LIMIT`), and
// modern Vue can render that without breaking a sweat. The viewport
// can be re-virtualized later if a captain hits a real memory
// ceiling, but the current bottleneck is correctness, not memory.

// ── Keyboard scroll ─────────────────────────────────────────────────
//
// The scroll container is a non-focusable `<div>`, so the browser's
// native PageUp / PageDown / Home / End handling never reaches it —
// those keys only act on the currently-focused element (or the
// document scroll, which the overlay layout doesn't have). We hook
// document-level keydown and translate the navigation keys into
// scroll operations on the transcript, BUT skip when the focus is in
// the composer / palette / any text input so editing keystrokes stay
// untouched. ~90% of the visible scroll viewport is one "page" — that
// matches the desktop convention (slightly less than full-screen so
// context overlaps).
const PAGE_OVERLAP_RATIO = 0.9

useEventListener(document, 'keydown', (ev: KeyboardEvent) => {
  if (isEditableTarget(ev.target)) {
    return
  }
  const el = scrollEl.value

  if (!el) {
    return
  }

  switch (ev.key) {
    case 'PageDown': {
      ev.preventDefault()
      el.scrollBy({ top: el.clientHeight * PAGE_OVERLAP_RATIO, behavior: 'smooth' })

      return
    }

    case 'PageUp': {
      ev.preventDefault()
      el.scrollBy({ top: -el.clientHeight * PAGE_OVERLAP_RATIO, behavior: 'smooth' })

      return
    }

    case 'Home': {
      // Bare Home jumps to top — `isEditableTarget` already gated
      // out inputs / textareas / contenteditable above, so the
      // textarea-cursor case is already safe. Ctrl/Cmd modifier is
      // also accepted for muscle memory parity with browsers.
      ev.preventDefault()
      el.scrollTo({ top: 0, behavior: 'smooth' })

      return
    }

    case 'End': {
      ev.preventDefault()
      el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' })
      // Smooth-scroll fires `scroll` events along the way, which our
      // `onScroll` handler reacts to — but the timing is browser-
      // dependent and on some phones the final tick lands AFTER the
      // browser settles, leaving a brief window where eviction could
      // be missed. Fire one more pass after a delay covering the
      // typical smooth-scroll duration.
      setTimeout(() => viewport.evictExtraPages(), 350)

      return
    }

    default:
      return
  }
})

// ── Backward pagination + eviction trigger ──────────────────────────
//
// One scroll handler drives two policies:
//
// 1. **Backward fetch** when scrollTop crosses
//    `LOAD_MORE_THRESHOLD_PX` — the captain is reading near the top
//    edge, pull older content.
// 2. **Page eviction** when the captain is within ~one viewport of
//    the bottom AND the cache exceeds `MAX_PAGES_KEPT`. Wider than
//    `useStickToBottom`'s strict 64px so eviction fires the moment
//    the captain returns to the live area, not only at the
//    absolute foot. The composable's eviction is idempotent — we
//    can safely call it on every scroll tick that satisfies the
//    near-bottom test; it's a no-op when the cache is already in
//    budget.
function onScroll(): void {
  const el = scrollEl.value

  if (!el) {
    return
  }
  // Gate every "auto" pagination action behind a captain-initiated
  // scroll. `useStickToBottom.onMounted` writes `scrollTop =
  // scrollHeight` on mount; the browser fires a synthetic scroll
  // event for that assignment + the subsequent MutationObserver
  // re-stick passes. Treat any scroll where the captain has NOT yet
  // pushed/dragged the bar themselves as bootstrap noise — neither
  // fetchNextPage nor evictExtraPages fire. The first delta over a
  // small threshold flips `hasUserScrolled` and unlocks normal
  // pagination. Threshold guards against jitter from the initial
  // anchor write.

  if (!hasUserScrolled.value) {
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight

    if (distanceFromBottom > LOAD_MORE_THRESHOLD_PX) {
      hasUserScrolled.value = true
    } else {
      return
    }
  }

  // Backward fetch.
  if (viewport.hasNextPage.value && !viewport.isFetchingNextPage.value && el.scrollTop < LOAD_MORE_THRESHOLD_PX) {
    void viewport.fetchNextPage()
  }

  // Eviction trigger — within one viewport of the bottom. Defer
  // the actual cache mutation to the next animation frame so the
  // DOM patch doesn't race the in-flight scroll gesture. When
  // eviction fires synchronously inside the scroll handler, the
  // resulting microtask removes nodes from the TOP of the DOM
  // while the browser is mid-gesture — scroll-anchoring can miss
  // (the anchor element itself may be evicted), and concurrent
  // `useStickToBottom` observers queue an rAF off the same DOM
  // mutation, doubling the disruption. rAF moves the mutation
  // out of the scroll-event task, after the browser has finished
  // processing the current tick. `evictExtraPages` is idempotent
  // — repeated rAF wrappers within a single gesture all observe
  // the same cache state; at most one mutates, subsequent calls
  // are within-budget no-ops.
  const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight

  if (distanceFromBottom <= el.clientHeight) {
    requestAnimationFrame(() => viewport.evictExtraPages())
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

/// Latest `updatedAt` across every entry in a block — turn / stream
/// / tool. The `tryMergeIntoExisting` path mutates the existing
/// entry in-place when streaming chunks arrive (`prev.entry.turn.text
/// += chunk; prev.entry.turn.updatedAt = it.seq`), so a v-memo dep
/// based purely on array LENGTH wouldn't change per chunk and the
/// live row's text would freeze. The latest `updatedAt` advances on
/// every merge — for the live block it ticks per chunk; for history
/// blocks it stays stable post-turn-end so v-memo skips render.
interface UpdatedAtBlock {
  turnEntries: { turn: { updatedAt?: number } }[]
  streamEntries: { item: { updatedAt?: number } }[]
  toolCalls: { call: { updatedAt?: number } }[]
  thoughts: { call: { updatedAt?: number } }[]
}

function latestUpdatedAt(block: UpdatedAtBlock): number {
  let max = 0

  for (const t of block.turnEntries) {
    const u = t.turn.updatedAt ?? 0

    if (u > max) {
      max = u
    }
  }

  for (const s of block.streamEntries) {
    const u = s.item.updatedAt ?? 0

    if (u > max) {
      max = u
    }
  }

  for (const tc of block.toolCalls) {
    const u = tc.call.updatedAt ?? 0

    if (u > max) {
      max = u
    }
  }

  for (const th of block.thoughts) {
    const u = th.call.updatedAt ?? 0

    if (u > max) {
      max = u
    }
  }

  return max
}

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
  <div class="chat-viewport-root">
    <div ref="scrollEl" class="chat-transcript" data-testid="chat-transcript" :data-instance-id="instanceId ?? ''" @scroll="onScroll">
      <Loading v-if="props.restoring" mode="scoped" status="restoring session — replaying transcript" />

      <slot v-if="isEmpty" name="empty" />

      <template v-else>
        <!-- Loading chip pinned at the top while a backward page is in
           flight. Sits outside the virtualized spacer so its height
           doesn't compete with row offsets. -->
        <div v-if="viewport.isFetchingNextPage.value" class="chat-load-chip animate-pulse" data-testid="chat-load-chip">loading earlier…</div>

        <!-- Plain v-for over `blocks`, with `v-memo` short-circuiting
           re-renders for history rows. Live row keeps re-rendering
           every chunk because `latestUpdatedAt(block)` advances per
           streaming merge (every chunk bumps the corresponding
           entry's `updatedAt` to the new seq). History rows have a
           stable `latestUpdatedAt` post-turn-end so v-memo skips
           their VNode walk entirely under streaming.
           Deps cover: turn identity, role, content-shape counts,
           per-chunk freshness via `latestUpdatedAt`, live flag,
           elapsed/usage labels. -->
        <Turn
          v-for="(block, blockIdx) in blocks"
          :key="block.groupKey"
          v-memo="[
            block.groupKey,
            block.role,
            block.turnEntries.length,
            block.toolCalls.length,
            block.streamEntries.length,
            latestUpdatedAt(block),
            blockIdx === liveBlockIdx,
            elapsedFor(block.turnId),
            usageFor(block.turnId)
          ]"
          :role="block.role"
          :live="blockIdx === liveBlockIdx"
          :elapsed="elapsedFor(block.turnId)"
          :usage="usageFor(block.turnId)"
        >
          <StreamCard
            v-if="combinedThoughtText(block).length > 0 || hasThinkingSignal(block)"
            :kind="StreamKind.Thinking"
            :active="blockIdx === liveBlockIdx"
            label="thought"
            :elapsed="thinkingElapsedFor(block)"
            :text="combinedThoughtText(block).length > 0 ? combinedThoughtText(block) : undefined"
          />
          <template v-for="entry in block.streamEntries" :key="`stream-${entry.createdAt}`">
            <StreamCard
              v-if="entry.item.kind === StreamItemKind.Plan"
              :kind="StreamKind.Planning"
              :active="blockIdx === liveBlockIdx"
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

          <ToolChips v-if="block.toolCalls.length > 0" :views="block.toolCalls.map((t) => format(t.call, adapterForActive))" />

          <template v-for="entry in block.toolCalls" :key="`term-${entry.call.toolCallId}`">
            <TerminalCard v-if="terminalIdForCall(entry.call)" :terminal-id="terminalIdForCall(entry.call) ?? ''" :instance-id="instanceId" @cancel="emit('cancel')" />
          </template>

          <template v-for="entry in block.turnEntries" :key="`turn-${entry.createdAt}`">
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
      </template>
    </div>

    <!-- Floating scroll-to-bottom chevron. Lives inside the viewport
         (anchored to the chat scroller's bottom-right) so it's not
         coupled to whatever sits below the viewport (composer, queue,
         permission stack). Visible only when the captain has scrolled
         away from the bottom — `stuck` flips false the moment they
         move >64px above the foot. Click jumps to the live area AND
         immediately drops any extra pages the captain accumulated
         while scrolling up. -->
    <button v-if="!stuck" type="button" class="scroll-to-bottom" data-testid="scroll-to-bottom" aria-label="Scroll to latest" @click="goToBottom">
      <FaIcon :icon="faChevronDown" />
    </button>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* Wrapper takes the flex-1 + min-h-0 the parent expects of the
 * Viewport root, so the inner scroller can fill while leaving room
 * for the absolute-positioned floating chevron without scrolling
 * with the content. */
.chat-viewport-root {
  @apply relative flex min-h-0 flex-1 flex-col;
}

.chat-transcript {
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
  position: relative;
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

/* Floating chevron — bottom-right of the chat surface. Sits above
 * the inner scroller so it doesn't move with content. Compact so
 * it doesn't hog vertical space on mobile; rem-based so the
 * mobile root-bump still scales it. */
.scroll-to-bottom {
  position: absolute;
  bottom: 0.625rem;
  right: 0.625rem;
  width: 1.5rem;
  height: 1.5rem;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg-dim);
  border: 1px solid var(--theme-border-soft);
  cursor: pointer;
  font-size: 0.7rem;
  transition:
    background-color 120ms ease,
    color 120ms ease,
    transform 120ms ease;
  z-index: 5;
}

.scroll-to-bottom:hover {
  background-color: var(--theme-surface);
  color: var(--theme-fg);
}

.scroll-to-bottom:active {
  transform: translateY(1px);
}
</style>
