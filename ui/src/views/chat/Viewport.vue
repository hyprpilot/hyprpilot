<script setup lang="ts">
/**
 * Chat transcript viewport. Reads off `useChatViewport` and feeds the
 * resulting blocks into a plain `v-for` render.
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
 * `v-for` is stable and the daemon-side transcript ring is the only
 * history bound. Inactive instance viewports stay mounted and are
 * hidden with viewport-level CSS rather than row-level
 * `content-visibility`, because row-level visibility breaks the
 * scroll-anchor offset math documented in `Turn.vue`.
 *
 * **Stick-to-bottom**: `useStickToBottom` already runs a
 * MutationObserver + ResizeObserver pair on the scroll container and
 * scrolls to the tail on every mutation while `stuck` is true. No
 * extra Vue watcher needed.
 */
import { faChevronDown } from '@fortawesome/free-solid-svg-icons'
import { useEventListener, useNow } from '@vueuse/core'
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
  useAgentRegistry,
  useChatViewport,
  useScrollAnchor,
  useSessionInfo,
  useSnapshotHydration,
  useStickToBottom,
  useTurns,
  type PlanEntry,
  type WireToolCall,
  type InstanceId
} from '@composables'
import { format, formatDuration } from '@lib'

const props = withDefaults(
  defineProps<{
    /// Instance whose transcript this retained viewport renders.
    instanceId?: InstanceId
    /// Whether this retained viewport is currently visible/focusable.
    active?: boolean
    /// Captain's "session is restoring" gate — keeps the scoped
    /// `<Loading>` overlay painted on top while transcript replay is
    /// in flight.
    restoring?: boolean
  }>(),
  {
    active: true
  }
)

const emit = defineEmits<{
  /// Fires when the captain hits the per-tool-call cancel button on
  /// a live terminal — Overlay.vue routes this to `session_cancel`.
  cancel: []
  /// Fires when an attachment pill on a captain turn is clicked.
  'attachment-open': [attachment: import('@ipc').Attachment]
}>()

const instanceId = computed<InstanceId | undefined>(() => props.instanceId)
const scrollEl = ref<HTMLElement>()
/// Flipped on the first captain-initiated input gesture after mount.
/// Gesture latch used by stick/anchor release paths so synthetic
/// mount-time scrolls do not look like captain intent.
///
/// **Why a gesture listener, not a scroll-event heuristic.**
/// `useStickToBottom.onMounted` writes `scrollTop = scrollHeight` on
/// mount, and its MutationObserver re-stick passes write again on
/// every chunk while the head page renders. The browser fires
/// synthetic `scroll` events for each — indistinguishable from a
/// captain's drag if you only watch `scrollTop`. A distance-based
/// gate (the previous shape) had two failure modes: (1) on a small
/// upward drag, `distanceFromBottom < threshold` so the gate stayed
/// locked, burning the gesture; (2) on a large drag past the
/// threshold, the gate unlocked only after the scroll gesture had already
/// started, racing sticky/anchor release.
///
/// `wheel` / `touchstart` / `pointerdown` are pure intent signals —
/// stick-to-bottom never fires them. First one flips the gate so
/// later gesture-sensitive paths know the captain has interacted.
///
/// Resets to `false` on every retained viewport mount. Focus flips
/// no longer remount the viewport, so each instance keeps its own
/// latch and scroll position.
const hasUserScrolled = ref(false)

function markUserScrolled(): void {
  if (!hasUserScrolled.value) {
    hasUserScrolled.value = true
  }
}

// `wheel` with negative deltaY (or trackpad scroll-up) releases
// stick + anchor SYNCHRONOUSLY before WebKit2GTK's compositor
// delivers the async scroll event a few frames later. End-of-frame
// races where a chunk-driven rAF fired first cancel the captain's
// scroll silently without this. Downward wheel falls through to the
// same markUserScrolled intent without release.
useEventListener(
  scrollEl,
  'wheel',
  (ev: WheelEvent) => {
    if (ev.deltaY < 0) {
      releaseStickAndAnchor()
    } else {
      markUserScrolled()
    }
  },
  { passive: true }
)
useEventListener(scrollEl, 'touchstart', markUserScrolled, { passive: true })
useEventListener(scrollEl, 'pointerdown', markUserScrolled, { passive: true })

// Mobile / touch path: release stick on the FIRST touchmove. `touchmove`
// only fires when the captain's finger actually moves (a tap that
// completes without movement never fires it) — so this is the touch
// equivalent of the upward-wheel release on desktop. Required because
// of a MutationObserver-vs-touch race specific to mobile webviews:
//
//   1. Live chunk lands → `MutationObserver` → `scheduleStick` → rAF
//      queued (stuck is still true).
//   2. Captain's finger starts moving (upward swipe to read older).
//   3. rAF fires BEFORE the captain's first `scroll` event lands —
//      iOS Safari / Chrome Android throttle inertia scroll events
//      relative to the gesture, but the rAF clock is unaffected.
//      `scrollToBottom()` writes `scrollTop = scrollHeight` and the
//      captain's swipe is silently cancelled (the synthetic scroll
//      consumes `suppressNextScrollUpdate` + forces stuck=true again).
//   4. Captain's next scroll events fire AFTER the snap-back, with
//      `prevScrollTop` re-baselined at the foot — `movedUp` never
//      flips, stick stays engaged. Captain reports "I can't break
//      the lock on mobile."
//
// Releasing stick synchronously at the first `touchmove` closes the
// race the same way `wheel.deltaY<0` does on desktop: the queued rAF
// short-circuits at `!stuck.value` when it fires, so the snap-back
// never happens. Taps without movement don't fire `touchmove`, so the
// stick stays engaged through tap-to-open-attachment / tap-to-copy /
// long-press-context-menu interactions.
useEventListener(scrollEl, 'touchmove', releaseStickAndAnchor, { passive: true })

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

const { stuck, scrollToBottom, release: releaseStick } = useStickToBottom(scrollEl)

// Scroll-anchor primitive — tracks `{ rowSeq, offsetWithinRow }`
// for the topmost visible row when stuck=false. Re-locks scrollTop
// on every resize so streaming chunks, Shiki late-renders, image
// loads, and page prepends don't shift the captain's reading line.
// Replaces the previous `captureBeforeBackwardFetch` /
// `restoreAfterBackwardFetch` pair (which only covered the prepend
// case).
// Anchor is consumed internally by the composable (ResizeObserver
// re-lock). We only need the imperative escape hatches at this
// layer — release on gesture start, mark on our own writes.
const { releaseAnchor, markProgrammaticScroll } = useScrollAnchor(scrollEl, { stuck })

/// Per-mount "first hydration" latch. `useStickToBottom` runs a
/// `scrollToBottom` in its own `onMounted`, but that fires BEFORE
/// the chat snapshot lands — at that moment `viewport.items.value`
/// is empty and `scrollHeight === clientHeight`, so the assignment
/// is a no-op. The MutationObserver-driven re-stick that follows
/// catches most subsequent mutations, but a fully-cached snapshot
/// (the captain returns to a previously-focused instance and the
/// query cache is still warm) renders in one Vue tick with no
/// observable DOM mutation between empty + populated — the
/// `MutationObserver` callback runs, but `scrollHeight` may already
/// equal `scrollTop + clientHeight` from the prior assignment, so
/// `scheduleStick`'s rAF coalescing decides there's nothing to do.
///
/// Captain reported: switching instances via the palette list,
/// the chat lands part-way up instead of at the foot. This watcher
/// closes the gap explicitly — the first transition from "no items"
/// to "items present" per mount triggers an explicit
/// `scrollToBottom` after Vue has flushed the DOM, regardless of
/// whether the MutationObserver fired. Retained viewports mount once
/// per known instance, so this watcher fires once per instance.
let firstHydrationLanded = false

watch(
  () => [viewport.items.value.length, props.active] as const,
  async([count, active]) => {
    if (firstHydrationLanded || count === 0 || !active) {
      return
    }
    firstHydrationLanded = true
    await nextTick()
    // Two ticks: the first lets Vue flush the items[] update into
    // the DOM; the second covers any nested `<Turn>` / `<StreamCard>`
    // child watchers that run their own `nextTick` for layout
    // measurement (markdown render passes, syntax-highlight swaps).
    await nextTick()
    markProgrammaticScroll()
    scrollToBottom()
  },
  { immediate: true }
)

/// Synchronous "captain wants to scroll up" gate. Calls into the
/// composables to cancel any pending sticky rAF + flip `stuck =
/// false` BEFORE the input gesture initiates its scroll, AND drops
/// the anchor so a coincident resize-driven re-lock doesn't pull
/// the captain back to where the anchor was captured pre-gesture.
/// Same race-closing pattern in both composables — see the
/// `useScrollAnchor.releaseAnchor` doc + `useStickToBottom.release`
/// doc for why this synchronous escape exists.
function releaseStickAndAnchor(): void {
  markUserScrolled()
  releaseStick()
  releaseAnchor()
}

// `stuck` is the auto-scroll signal — 128px-from-bottom threshold
// (tuned in `useStickToBottom`) so a captain reading 1 viewport
// above the foot doesn't get yanked back on every chunk. There is no
// viewport-window eviction anymore; the daemon transcript ring is the
// only history bound.

// **Window-focus → scroll-to-end, BUT only when the captain was
// already stuck at the foot.** Tauri 2 propagates window focus to the
// DOM `focus` event; browser tabs fire it natively. Unconditionally
// snapping on focus yanks the captain off whatever older message they
// were reading the moment they alt-tab back — exactly the
// scroll-hostage symptom they reported. Gate on `stuck.value` so the
// snap only fires when they were AT the foot before they switched
// away (which is the only state where a snap is the right answer
// anyway: a streaming chunk could have pushed `scrollHeight` while
// the tab was hidden, leaving a small gap that the snap re-closes).
useEventListener(window, 'focus', () => {
  if (!stuck.value) {
    return
  }
  void nextTick(() => {
    markProgrammaticScroll()
    scrollToBottom()
  })
})

/// Floating chevron click — jump straight to the foot. No page
/// eviction needed (pagination-eviction was removed in this PR —
/// the cache now holds every replayed page for the lifetime of the
/// instance, so there's nothing to trim before the jump).
function goToBottom(): void {
  markProgrammaticScroll()
  scrollToBottom()
}

// ── Rendering ──────────────────────────────────────────────────────
//
// **No virtualization and no lazy viewport window.** The initial
// snapshot asks for the daemon's retained transcript ring. The plain
// `v-for` keeps history in DOM for the active viewport; inactive
// viewports are hidden at the root with CSS so focus switches do not
// delete viewport-local scroll/anchor state.

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
  <div class="chat-viewport-root" :data-active="props.active ? 'true' : 'false'">
    <div ref="scrollEl" class="chat-transcript" data-testid="chat-transcript" :data-instance-id="instanceId ?? ''">
      <Loading v-if="props.restoring" mode="scoped" status="restoring session — replaying transcript" />

      <slot v-if="isEmpty" name="empty" />

      <template v-else>
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
          :anchor-seq="block.startedAt"
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
              :stats="entry.item.stats"
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
            <template v-if="entry.turn.role === TurnRole.Agent">
              <Body v-if="entry.turn.text.length > 0" :role="Role.Assistant" :text="entry.turn.text" markdown />
              <!-- Agent attachments (`AgentAttachment` transcript items)
                 rendered here. nvim's `render_attachment` handles this
                 explicitly; the Vue UI was silently dropping the
                 attachments array on agent turns because the previous
                 template only rendered `<Attachments>` in the user branch. -->
              <Attachments
                v-if="entry.turn.attachments && entry.turn.attachments.length > 0"
                :attachments="entry.turn.attachments"
                @open="(att) => emit('attachment-open', att)"
              />
            </template>
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
         move >128px above the foot. Click jumps to the live area and
         resumes auto-follow. -->
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

.chat-viewport-root[data-active='false'] {
  position: absolute;
  inset: 0;
  visibility: hidden;
  pointer-events: none;
  content-visibility: hidden;
  contain: layout style paint;
}

.chat-viewport-root[data-active='true'] {
  position: relative;
  visibility: visible;
  content-visibility: visible;
}

.chat-transcript {
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
  position: relative;
  padding: 0 1rem 0 0.25rem;
  /* Disable browser-native scroll anchoring — `useScrollAnchor`
   * owns the re-lock path. With both active, the browser's
   * opaque heuristic compensation would race our explicit
   * `scrollTop = newTop + offsetWithinRow` write and produce
   * a visible double-shift. JS owns the anchoring. */
  overflow-anchor: none;
}

/* Floating chevron — bottom-right of the chat surface. Sits above
 * the inner scroller so it doesn't move with content. Compact so
 * it doesn't hog vertical space on mobile; rem-based so the
 * mobile root-bump still scales it. */
.scroll-to-bottom {
  position: absolute;
  bottom: 0.625rem;
  right: 0.625rem;
  width: 2rem;
  height: 2rem;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  background-color: var(--theme-surface);
  color: var(--theme-accent);
  border: 1px solid var(--theme-accent);
  box-shadow: 0 0.375rem 1.25rem rgba(0, 0, 0, 0.35);
  cursor: pointer;
  font-size: 0.7rem;
  transition:
    background-color 120ms ease,
    color 120ms ease,
    transform 120ms ease;
  z-index: 5;
}

.scroll-to-bottom:hover {
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
}

.scroll-to-bottom:active {
  transform: translateY(1px);
}
</style>
