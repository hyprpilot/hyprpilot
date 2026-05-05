<script setup lang="ts">
/**
 * Overlay shell — the single page-level view the Tauri webview mounts
 * (see `App.vue`). Composes the K-250 chat primitives into the running app.
 *
 * Frame slots (see `components/Frame.vue`):
 *   default slot  — transcript body. `<Turn>` blocks built from
 *                   `useTranscript` + `useStream` + `useTools`, followed
 *                   by `<PermissionStack>` fed from
 *                   `useAdapter().lastPermission`.
 *   #composer     — `<Composer>` wired to `useAdapter().submit`.
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
import { faPenToSquare } from '@fortawesome/free-solid-svg-icons'
import { useNow } from '@vueuse/core'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'

import {
  type BreadcrumbCount,
  Button,
  ButtonTone,
  ButtonVariant,
  type ComposerPill,
  ComposerPillKind,
  Loading,
  Modal,
  ModalDescription,
  ModalInput,
  Phase,
  PlanStatus,
  type QueuedMessage,
  Role,
  StreamKind,
  Toast,
  ToastTone,
  type PlanItem
} from '@components'
import {
  pushToast,
  dispatchQueueHead,
  dispatchQueueItem,
  popQueueItem,
  pushToQueue,
  pushToQueueAt,
  removeFromQueue,
  startActiveInstance,
  useStickToBottom,
  startQueueDispatcher,
  stopQueueDispatcher,
  StreamItemKind,
  TurnRole,
  useActiveInstance,
  useAdapter,
  useAgentRegistry,
  resetPermissions,
  useAttachments,
  useDaemonCwd,
  useHomeDir,
  useKeymap,
  useKeymaps,
  usePalette,
  usePermissions,
  useRenameInstanceModal,
  usePhase,
  useProfiles,
  useQueue,
  useSessionHistory,
  useSessionInfo,
  useTimelineBlocks,
  useToasts,
  useTurns,
  type KeymapEntry,
  startSessionStream,
  type InstanceId,
  type PlanEntry,
  type WireToolCall
} from '@composables'
import { type Attachment, invoke, Modifier, TauriCommand } from '@ipc'
import { format, formatDuration, log } from '@lib'
import { Attachments, Body as ChatBody, ChangeBanner, PermissionModal, StreamCard, TerminalCard, ToolChips, Turn } from '@views/chat'
import { Composer, PermissionStack, QueueStrip } from '@views/composer'
import { Frame } from '@views/header'
import { IdleScreen } from '@views/idle'
import { CommandPalette, commitInstanceRename, isPaletteLeafId, openRootLeaf, openRootPalette, PaletteLeafId, validateInstanceName } from '@views/palette'

const { submit, cancel } = useAdapter()
const { adapterFor } = useAgentRegistry()
const { pending: pendingAttachments, clear: clearAttachments } = useAttachments()
const { phase } = usePhase()
const { profiles, selected: selectedProfile } = useProfiles()
const activeAgentId = computed(() => profiles.value.find((p) => p.id === selectedProfile.value)?.agent)
// Session history is wired but the overlay shell doesn't surface a
// session picker yet — keeping the binding live so the backend stays
// warm; the palette view (K-249) takes over this role. The list count
// rides on the row-2 sessions breadcrumb pill.
const { sessions: sessionList, load: restoreSession } = useSessionHistory(activeAgentId, selectedProfile)

// LFG idle landing only previews the most-recent few sessions —
// rendering the full registry inline pushes the wordmark + kbd
// legend off-screen on small anchors. The full list lives behind
// the sessions palette leaf (Ctrl+K → sessions). Cap matches the
// "couple of sessions" intent — small enough to fit alongside the
// LFG accent + kbd legend at every supported overlay width.
const IDLE_SESSIONS_PREVIEW = 5

// Filter to sessions matching the daemon's current cwd. The idle
// screen answers "what would I resume here in this directory?" —
// showing sessions from sibling projects pollutes the answer. The
// sessions palette (Ctrl+K → sessions) still surfaces the full
// registry for cross-cwd navigation.
const sessionsForCwd = computed(() => {
  const cwd = sessionInfo.value.cwd ?? daemonCwd.value

  if (cwd === undefined) {
    return sessionList.value
  }

  return sessionList.value.filter((s) => s.cwd === cwd)
})
const sessionListPreview = computed(() => sessionsForCwd.value.slice(0, IDLE_SESSIONS_PREVIEW))

// Idle-row click → resume that session. `restoreSession` mints a
// fresh instance UUID, fires `session_load`, and the daemon-side
// `registry.focus(...)` flips the active instance onto the resumed
// one so replay events paint into the visible transcript. No-op
// when the row carries no `id` (defensive — every ACP `SessionInfo`
// should but the type is `id?`).
function onRestoreSessionClick(sessionId: string | undefined): void {
  if (!sessionId) {
    return
  }
  void restoreSession(sessionId)
}

const { id: activeInstanceId, count: instancesCount } = useActiveInstance()
// useTranscript / useStream / useTools wired through useTimelineBlocks
// — accessing them here would just allocate redundant computeds. The
// idle-screen branch reads `timelineBlocks.length === 0` for the
// no-content gate.
const { rowQueue: permissionRowQueue, modalQueue: permissionModalQueue, respond: respondPermission } = usePermissions()
const { openTurnId, turns: turnRecords } = useTurns()
const { info: sessionInfo } = useSessionInfo()
const { displayPath } = useHomeDir()
const { daemonCwd } = useDaemonCwd()
const { items: queuedItems, flush: flushActiveQueue } = useQueue()
const { blocks: timelineBlocks } = useTimelineBlocks()
const { entries: toastEntries, dismiss: dismissToast } = useToasts()
const activeToast = computed(() => toastEntries.value[0])

const queueRows = computed<QueuedMessage[]>(() => queuedItems.value.map((q) => ({ id: q.id, text: q.text })))

const transcriptEl = ref<HTMLElement>()

useStickToBottom(transcriptEl)

const sending = ref(false)
const composerRef = ref<InstanceType<typeof Composer>>()

const activeProfile = computed(() => profiles.value.find((p) => p.id === selectedProfile.value))

/**
 * Header + idle-banner cwd: prefer the active session's cwd when one
 * has reported in, fall back to the daemon's own cwd so the captain
 * sees where the next instance will land before any session info
 * arrives. Without the fallback the header pill renders blank for
 * the entire pre-first-turn window — captain reads that as "no cwd
 * configured" instead of "I'll spawn where the daemon was started".
 *
 * `displayPath` does the `home → ~` substitution (read-only inverse
 * of the daemon's `paths_resolve`); chrome's CSS `text-overflow:
 * ellipsis` handles overflow.
 */
const headerCwd = computed<string | undefined>(() => {
  const raw = sessionInfo.value.cwd ?? daemonCwd.value

  return raw ? displayPath(raw) : undefined
})

// Untruncated home-shortened path for the cwd button's tooltip —
// same display form as `headerCwd` since CSS already does the
// trimming. Kept as a separate ref in case future chrome diverges
// (e.g. tooltip wants the original absolute path).
const headerCwdFull = computed<string | undefined>(() => {
  const raw = sessionInfo.value.cwd ?? daemonCwd.value

  return raw ? displayPath(raw) : undefined
})

const idleCwd = computed<string | undefined>(() => {
  const raw = sessionInfo.value.cwd ?? daemonCwd.value

  return raw ? displayPath(raw) : undefined
})

const headerCounts = computed<BreadcrumbCount[]>(() => [
  {
    id: PaletteLeafId.Mcps,
    label: 'mcps',
    count: sessionInfo.value.mcpsCount
  },
  {
    id: PaletteLeafId.Sessions,
    label: 'sessions',
    count: sessionsForCwd.value.length
  }
])

function onPillClick(target: 'profile' | 'mode' | 'provider'): void {
  switch (target) {
    case 'profile':
      openRootLeaf(PaletteLeafId.Profiles)

      return

    case 'mode':
      openRootLeaf(PaletteLeafId.Modes)

      return

    case 'provider':
      openRootLeaf(PaletteLeafId.Models)

      return
  }
}

function onBreadcrumbClick(id: string): void {
  if (!isPaletteLeafId(id)) {
    return
  }

  if (id === PaletteLeafId.Mcps) {
    const instanceId = activeInstanceId.value

    if (!instanceId) {
      openRootLeaf(id)

      return
    }
    openRootLeaf(id, { mcps: { instanceId } })

    return
  }
  openRootLeaf(id)
}

function onToggleCwd(): void {
  openRootLeaf(PaletteLeafId.Cwd)
}

// Header X — hide the overlay, leaving the daemon + every live
// instance running. Toggle is the only command we expose; the X is
// only visible while the overlay is mapped, so toggling here always
// hides.
async function onCloseOverlay(): Promise<void> {
  try {
    await invoke(TauriCommand.WindowToggle)
  } catch(err) {
    log.warn('overlay: window_toggle failed', { err: String(err) })
  }
}

// Block grouping is driven by ACP turn ids (Rust mints one per
// `session/prompt` and stamps every notification it emits with that
// id; see `acp:turn-started` / `acp:turn-ended`). Assistant entries
// carrying the same `turnId` collapse into one block; user turns —
// which arrive before any `TurnStarted` for the reply — sit in their
// own per-message block.
//
// Implementation lives in `composables/instance/use-timeline-blocks`
// (S2 + S8). Overlay reads the block list as a hierarchy that
// mirrors the user's mental model — "what happened during this turn".

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

// Reactive wall-clock ref that re-renders elapsed labels every second
// while a turn / thought is in flight. Once the matching `endedAtMs`
// (turn) or `completedAtMs` (per-tool) lands, the label converges to
// the daemon-stamped value and stops moving — `now` keeps ticking but
// the label expression switches branches.
const liveNow = useNow({ interval: 1000 })

function liveNowMs(): number {
  return liveNow.value.getTime()
}

const turnDurationLabels = computed<Map<string, string>>(() => {
  const out = new Map<string, string>()
  const now = liveNowMs()

  for (const t of turnRecords.value) {
    // `startedAtMs === 0` is the daemon's "no real timing" signal —
    // synthetic / replay turns don't have a meaningful wall clock.
    // Skip them so the chip doesn't render replay-processing time as
    // if it were the turn duration.
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

/// Thinking elapsed for the assistant block. Sum of:
///   1. Per-turn stream-shape thinking — `TurnRecord.thinkingMs`
///      (closed intervals) + `(now - thinkingOpenAtMs)` while the
///      agent is actively reasoning. The accumulator pauses on
///      `agent_message_chunk` / `tool_call` and resumes on the next
///      `agent_thought_chunk`, so the captain reads true reasoning
///      time, not "wall clock until agent finished writing".
///   2. Per-tool kind=think durations from `block.thoughts`
///      (codex-style: each kind=think tool call carries its own
///      started_at / completed_at).
/// Returns `undefined` when no thought signal exists (no card).
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
      // Defensive: a TurnRecord pushed before this composable's
      // HMR reload may not have `thinkingMs` set — `undefined + N`
      // would cascade `NaN` into `formatDuration` and throw inside
      // `intervalToDuration`, which fails the StreamCard's prop
      // binding and silently drops the thought card.
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

/// Render the thinking card whenever the agent is reasoning, even if
/// every chunk so far has carried empty text (claude-code-acp emits
/// `agent_thought_chunk` with `text: ""` for content_block_start
/// before any deltas land — the card should appear immediately).
/// Two truthy signals: real prose accumulated OR the per-turn
/// thinking interval has any time recorded / is currently open.
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

let stopStream: (() => void) | undefined

function firePermission(action: 'allow' | 'deny'): void {
  // TODO(K-281 follow-up): Tab = next row cycling. Today the approval
  // keybind always addresses the oldest-active (first non-queued) prompt.
  const active =
    permissionRowQueue.value.find((v) => !v.queued) ?? permissionRowQueue.value[0] ?? permissionModalQueue.value.find((v) => !v.queued) ?? permissionModalQueue.value[0]

  if (!active) {
    return
  }
  // Keybind maps to the basic-once variant ONLY: `allow` → exact
  // `allow_once`, `deny` → exact `reject_once`. The "always" variants
  // mutate the trust store across sessions — too destructive to bind
  // a single keystroke to. If the agent didn't offer the basic option
  // (rare; some plan-mode prompts only offer `allow_once_with_*`
  // shapes), surface a toast and refuse — typing the wrong key silently
  // committing an "always" decision is the worst possible outcome.
  const targetKind = action === 'allow' ? 'allow_once' : 'reject_once'
  const opt = active.options.find((o) => o.kind === targetKind)

  if (!opt) {
    log.info('keybind no-op', {
      action, target: 'permission', reason: 'no_basic_variant', offered: active.options.map((o) => o.kind)
    })
    pushToast(ToastTone.Warn, `${action} keybind: agent didn't offer ${targetKind}; click an option directly`)

    return
  }
  log.info('keybind invoked', {
    action, target: 'permission', optionId: opt.optionId, kind: opt.kind
  })
  void onPermissionReply(active.request.requestId, opt.optionId)
}

const { keymaps } = useKeymaps()
const { closeAll: closeAllPalettes } = usePalette()

// Singleton "rename instance" modal target. The palette's
// `instance > rename` action populates `target`; the modal v-ifs
// off it. Save / cancel reset to undefined → modal unmounts.
const renameModal = useRenameInstanceModal()
const renameDraft = ref('')

watch(
  () => renameModal.target.value,
  (next) => {
    renameDraft.value = next?.currentName ?? ''
  },
  { immediate: true }
)

async function onRenameAccept(): Promise<void> {
  const target = renameModal.target.value

  if (!target) {
    return
  }
  const ok = await commitInstanceRename(target.instanceId, renameDraft.value)

  if (ok) {
    renameModal.close()
  }
}

function onRenameCancel(): void {
  renameModal.close()
}

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

          return true
        }
      },
      {
        binding: keymaps.value.approvals.deny,
        handler: () => {
          firePermission('deny')

          return true
        }
      },
      {
        // Cancel-current-turn — Ctrl+C by default. Mirrors the
        // composer's stop button + the shell convention. Always
        // fires regardless of phase: after a session restore the
        // phase resolves to Idle (no open turn), but the user may
        // still want to send a CancelNotification to clear any
        // server-side in-flight state inherited from the suspended
        // session. The daemon's `session_cancel` is a soft-fail
        // when there's nothing to cancel — no harm.
        binding: keymaps.value.chat.cancel_turn,
        handler: () => {
          void onCancel()

          return true
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
      {
        // Direct-jump to the instances palette — captain's "what
        // else is running / kill an instance" panel. The only
        // sub-palette we still ship a dedicated focus bind for.
        binding: keymaps.value.palette.instances?.focus ?? { modifiers: [Modifier.Ctrl], key: 'i' },
        handler: () => {
          log.info('keybind invoked', { action: 'focus', target: 'palette.instances' })
          openRootLeaf(PaletteLeafId.Instances)

          return true
        }
      },
      {
        // Toggle the overlay's visibility — same surface as the
        // tray's "show/hide" + the `window/toggle` RPC. While
        // visible, hides; while hidden, this binding can't fire
        // anyway (no webview keyboard input) so the bind is
        // effectively show-only-from-Hyprland-bind / hide-from-here.
        //
        // Fallback to a hardcoded Ctrl+Q when the wire-loaded
        // keymap doesn't carry `window.toggle` (older daemon binary
        // without the [keymaps.window] defaults). User config can
        // still override the default through the wire shape.
        binding: keymaps.value.window?.toggle ?? { modifiers: [Modifier.Ctrl], key: 'q' },
        handler: () => {
          log.info('keybind invoked', { action: 'toggle', target: 'window' })
          void invoke(TauriCommand.WindowToggle).catch((err: unknown) => {
            log.warn('window_toggle failed', { err: String(err) })
          })

          return true
        }
      },
      {
        // Dispatch the head of the active instance's submit queue.
        // Captain-only — the queue never auto-drains on turn end.
        // Falls back to the hardcoded default when the wire-loaded
        // keymap predates the field.
        binding: keymaps.value.queue?.send ?? { modifiers: [Modifier.Ctrl], key: 'enter' },
        handler: () => {
          const instanceId = activeInstanceId.value

          if (!instanceId) {
            return true
          }
          log.info('keybind invoked', { action: 'send', target: 'queue' })
          dispatchQueueHead(instanceId)

          return true
        }
      },
      {
        // Drop the head of the queue without sending. Pairs with the
        // strip's drop button; useful when typing reveals the queued
        // entry was a misfire.
        binding: keymaps.value.queue?.drop ?? { modifiers: [Modifier.Ctrl], key: 'backspace' },
        handler: () => {
          const instanceId = activeInstanceId.value

          if (!instanceId) {
            return true
          }
          const head = queuedItems.value[0]

          if (!head) {
            return true
          }
          log.info('keybind invoked', {
            action: 'drop',
            target: 'queue',
            queuedItemId: head.id
          })
          removeFromQueue(instanceId, head.id)

          return true
        }
      }
    ]
  }
)

let stopActiveInstanceStore: (() => void) | undefined

/**
 * Window-level capture-phase listener for the visibility toggle. Runs
 * BEFORE every other keydown listener (textarea, document, palette);
 * cannot be swallowed by an earlier handler's stopPropagation. The
 * config-driven keymap entry above stays as the customisation surface
 * for users who override the binding; this is the always-on path so
 * `window/toggle` is reachable even when the wire-loaded keymap lacks
 * the field, the textarea is focused, or another handler eats the
 * bubble phase.
 */
function windowToggleCaptureListener(e: KeyboardEvent): void {
  if (e.type !== 'keydown') {
    return
  }

  if (!e.ctrlKey || e.shiftKey || e.altKey || e.metaKey) {
    return
  }

  if (e.key.toLowerCase() !== 'q') {
    return
  }
  e.preventDefault()
  e.stopPropagation()
  log.info('keybind invoked', {
    action: 'toggle',
    target: 'window',
    via: 'capture'
  })
  void invoke(TauriCommand.WindowToggle)
    .then((visible) => {
      log.info('window_toggle ok', { visible: String(visible) })
    })
    .catch((err: unknown) => {
      log.warn('window_toggle failed', { err: String(err) })
      pushToast(ToastTone.Err, `window_toggle failed: ${String(err)}`)
    })
}

onMounted(async() => {
  window.addEventListener('keydown', windowToggleCaptureListener, { capture: true })
  startQueueDispatcher()

  try {
    stopActiveInstanceStore = await startActiveInstance()
  } catch(err) {
    log.error('invoke failed', { command: 'startActiveInstance' }, err)
    pushToast(ToastTone.Err, `active-instance bind failed: ${String(err)}`)
  }

  try {
    stopStream = await startSessionStream()
  } catch(err) {
    log.error('invoke failed', { command: 'startSessionStream' }, err)
    pushToast(ToastTone.Err, `stream bind failed: ${String(err)}`)
  }
})

onUnmounted(() => {
  window.removeEventListener('keydown', windowToggleCaptureListener, { capture: true })
  stopStream?.()
  stopStream = undefined
  stopActiveInstanceStore?.()
  stopActiveInstanceStore = undefined
  stopQueueDispatcher()
})

// Skill attachments are per-turn but tied to the active instance —
// switching to another instance mid-compose discards any pending
// picks (they were assembled against the previous instance's context).
watch(activeInstanceId, (next, prev) => {
  if (prev && next !== prev) {
    clearAttachments()
  }
})

// Pull the agent-supplied terminal id off a tool call's rawInput so
// the timeline can render an inline `ChatTerminalCard` next to the
// chip. Bash + Terminal tool variants both surface `terminal_id`
// when the agent allocates one.
function terminalIdForCall(call: { rawInput?: Record<string, unknown> }): string | undefined {
  const raw = call.rawInput

  if (!raw) {
    return undefined
  }
  const candidate = raw.terminal_id ?? raw.terminalId

  return typeof candidate === 'string' && candidate.length > 0 ? candidate : undefined
}

// claude-code-acp serializes thinking as a `tool_call` with kind:
// "think" rather than as `agent_thought_chunk` session-update — so
// the thought body lives on `content[].text` (the tool-call text
// blocks) plus the chip's `title` as a one-line summary. Stitch
// them: title leads, content paragraphs follow.
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

/**
 * Merge every thought signal in a block into a single ordered string
 * so the chat surface renders one thinking card per turn instead of
 * stacking N. Both wire shapes feed in:
 *
 *   - tool-call thoughts (`block.thoughts`) — claude-code-acp emits
 *     each thinking-block as its own `tool_call` with `kind=think`.
 *     One turn can carry many.
 *   - stream-side thoughts (`block.streamEntries` of kind Thought) —
 *     `agent_thought_chunk` notifications; some agents prefer this
 *     channel.
 *
 * Order by `createdAt` so the captain sees them in the order the
 * agent emitted them. Empty result → no card rendered.
 */
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

    // Keep whitespace-only chunks too — during streaming a thought
    // item may briefly hold a leading newline / space that the next
    // delta replaces. Filtering by `.trim().length` was hiding the
    // card during those windows; the band's existence carries signal
    // even before the prose lands.
    if (text.length > 0) {
      merged.push({ createdAt: entry.createdAt, text })
    }
  }
  merged.sort((a, b) => a.createdAt - b.createdAt)

  return merged.map((m) => m.text).join('\n\n')
}

async function onAttachmentOpen(att: Attachment): Promise<void> {
  if (!att.path) {
    return
  }

  try {
    const { open } = await import('@tauri-apps/plugin-shell')

    await open(att.path)
  } catch(err) {
    log.warn('attachments: open failed', { path: att.path, err: String(err) })
    pushToast(ToastTone.Err, `couldn't open ${att.path}`)
  }
}

async function onCancel(): Promise<void> {
  const instanceId = activeInstanceId.value

  log.info('cancel turn requested', { instanceId })

  // Clear local permission state immediately so the user gets
  // instant feedback. The daemon `session_cancel` sends an ACP
  // CancelNotification, but the agent's response (or lack thereof)
  // can lag; without a local clear, a stuck permission stack from
  // a restored session would stay visible until the agent obliged.
  if (instanceId) {
    resetPermissions(instanceId)
  }
  pushToast(ToastTone.Warn, 'cancel sent')

  try {
    await cancel({ instanceId })
  } catch(err) {
    log.error('invoke failed', { command: 'session_cancel' }, err)
    pushToast(ToastTone.Err, `cancel failed: ${String(err)}`)
  }
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

/// One modal at a time — permission UI is blocking by nature, so
/// stacking doesn't add value. Subsequent modal-class prompts wait
/// behind this one in `permissionModalQueue`.
const activeModalView = computed(() => permissionModalQueue.value[0])

async function onPermissionReply(requestId: string, optionId: string): Promise<void> {
  log.info('permission click', { requestId, optionId })

  try {
    await respondPermission(requestId, optionId)
  } catch(err) {
    log.error(
      'invoke failed',
      {
        command: 'permission_reply',
        requestId,
        optionId
      },
      err
    )
    pushToast(ToastTone.Err, `permission reply failed: ${String(err)}`)
  }
}

// Mint a fresh instance UUID the backend will adopt. Matches
// `AcpInstances::InstanceKey` — a v4 UUID, opaque to the UI except
// for identity. Used when there's no active instance yet (first
// submit, or after a "new session" reset).
function mintInstanceId(): InstanceId {
  return crypto.randomUUID()
}

/**
 * Project image-attachment composer pills onto the wire
 * `Attachment` shape so the daemon's `build_prompt_blocks` can
 * emit `ContentBlock::Image` for each one. Skill-style pills
 * (palette-pushed onto `useAttachments`) skip this path — they
 * already arrive as wire `Attachment`s with `body` set.
 *
 * `path` is synthesized from the optional `fileName` (drag-drop /
 * file picker) or the MIME's extension (clipboard paste) so the
 * Rust side's `mime_guess` fallback still works on the extension
 * if the explicit `mime` field is ever stripped en route.
 */
function imagePillsToAttachments(pills: ComposerPill[]): Attachment[] {
  return pills
    .filter((p) => p.kind === ComposerPillKind.Attachment && p.mimeType?.startsWith('image/'))
    .map((p) => {
      const ext = (p.mimeType ?? 'image/png').split('/')[1] ?? 'png'
      const synthName = p.fileName && p.fileName.length > 0 ? p.fileName : `${p.id}.${ext}`

      return {
        slug: p.id,
        path: synthName,
        body: '',
        title: p.label,
        data: p.data,
        mime: p.mimeType
      }
    })
}

/**
 * Tracks an in-flight queue-edit round-trip. Set when the captain
 * clicks the queue strip's edit button: the entry leaves the queue,
 * its text + pills land in the composer, and `position` remembers
 * the original slot. On the next submit we re-insert at the same
 * position so order is preserved.
 */
const editingQueueSlot = ref<{ instanceId: InstanceId; position: number } | undefined>(undefined)

function onSubmit(payload: { text: string; attachments: ComposerPill[] }): void {
  const { text, attachments } = payload
  // Skill / resource attachments live in the `useAttachments` singleton
  // (K-268 palette pushes onto it). They snapshot at submit time so a
  // resubmit after cancel sends the same set; submit-ack clears.
  const skillAttachments = [...pendingAttachments.value]
  // Image pills (paperclip / drag-drop / Ctrl+P) project onto the
  // wire `Attachment` shape with `data` + `mime` set; the daemon's
  // `build_prompt_blocks` dispatches those to ACP `ContentBlock::Image`
  // (versus skill resources which use `body` + `ContentBlock::Resource`).
  const imageAttachments = imagePillsToAttachments(attachments)
  const wireAttachments = [...skillAttachments, ...imageAttachments]

  log.info('composer submit', {
    text_len: text.length,
    image_attachments: imageAttachments.length,
    skill_attachments: skillAttachments.length,
    profileId: selectedProfile.value,
    editing: editingQueueSlot.value !== undefined
  })

  const instanceId = activeInstanceId.value ?? mintInstanceId()

  useActiveInstance().set(instanceId)

  // Edit-resubmit: a captain pulled this entry into the composer
  // via the queue-strip edit button. Land it back in the queue at
  // the original slot regardless of phase — the queue is captain-
  // drained today (Ctrl+Enter or per-row send), so always
  // re-queueing keeps the order predictable.
  const editing = editingQueueSlot.value

  if (editing && editing.instanceId === instanceId) {
    pushToQueueAt(instanceId, editing.position, {
      text,
      pills: attachments,
      skillAttachments
    })
    editingQueueSlot.value = undefined
    composerRef.value?.clear()
    clearAttachments()

    return
  }

  // Submit-while-busy: queue at the tail. The queue never auto-
  // drains today — captain dispatches via Ctrl+Enter or the per-
  // row send button on the queue strip.
  if (phase.value !== Phase.Idle) {
    pushToQueue(instanceId, {
      text,
      pills: attachments,
      skillAttachments
    })
    composerRef.value?.clear()
    clearAttachments()

    return
  }

  sending.value = true
  // The user turn lands as a daemon-emitted `TranscriptItem::UserPrompt`
  // event; the demuxer in `use-session-stream` routes it through to
  // `pushTranscriptChunk`. No optimistic mirror here — daemon is the
  // single source of truth.
  submit({
    text,
    instanceId,
    profileId: selectedProfile.value,
    attachments: wireAttachments
  })
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

function onQueueDrop(id: string): void {
  if (!activeInstanceId.value) {
    return
  }
  removeFromQueue(activeInstanceId.value, id)
}

function onQueueDropAll(): void {
  flushActiveQueue()
}

/**
 * Pull a queued entry into the composer. Snapshot the slot so a
 * subsequent submit (Enter / send button) re-inserts at the same
 * position — order is preserved end-to-end. The composer's
 * existing draft is replaced wholesale; users committed to a
 * different message can drop it via the queue's drop button.
 */
function onQueueEdit(itemId: string): void {
  const instanceId = activeInstanceId.value

  if (!instanceId) {
    return
  }
  const popped = popQueueItem(instanceId, itemId)

  if (!popped) {
    return
  }
  editingQueueSlot.value = { instanceId, position: popped.position }
  composerRef.value?.setDraft({
    text: popped.item.text,
    pills: popped.item.pills
  })
  log.info('queue edit', {
    instanceId,
    queuedItemId: itemId,
    slot: popped.position
  })
}

/**
 * Per-row "send now" — pop the specific entry out of the queue
 * and dispatch it via the adapter. Skips the head if the captain
 * picked a later row.
 */
function onQueueSend(itemId: string): void {
  const instanceId = activeInstanceId.value

  if (!instanceId) {
    return
  }
  log.info('queue send-now', { instanceId, queuedItemId: itemId })
  dispatchQueueItem(instanceId, itemId)
}
</script>

<template>
  <Frame
    :profile="sessionInfo.profileId ?? sessionInfo.agent ?? 'none'"
    :name="sessionInfo.name"
    :phase="phase"
    :mode-tag="sessionInfo.mode"
    :provider="sessionInfo.agent"
    :model="sessionInfo.model"
    :title="sessionInfo.title"
    :cwd="headerCwd"
    :cwd-full="headerCwdFull"
    :counts="headerCounts"
    :instances-count="instancesCount"
    :git-status="sessionInfo.gitStatus"
    @pill-click="onPillClick"
    @breadcrumb-click="onBreadcrumbClick"
    @toggle-cwd="onToggleCwd"
    @close="onCloseOverlay"
    @instances-click="openRootLeaf(PaletteLeafId.Instances)"
  >
    <template v-if="activeToast" #toast>
      <Toast :tone="activeToast.tone" :body="activeToast.body" @dismiss="dismissToast(activeToast.id)" />
    </template>

    <div ref="transcriptEl" class="chat-transcript" data-testid="chat-transcript" :data-instance-id="activeInstanceId ?? ''">
      <!-- Scoped <Loading> overlay during `session/load` replay.
           `sessionInfo.restoring` flips true on user-initiated
           `loadSession` and clears on the first TurnEnded for the
           resumed instance (daemon auto-cancels after the load,
           guaranteeing one). The header / composer / palette stay
           operational behind the cover so the user can still
           cancel, switch instance, or open Ctrl+K. Sits as a
           sibling of `.chat-transcript-inner` so the cover spans
           the full transcript box including the gutter padding —
           the inner div carries the gutters. -->
      <Loading v-if="sessionInfo.restoring" mode="scoped" status="restoring session — replaying transcript" />
      <div class="chat-transcript-inner">
        <!-- idle landing — paints when the chat surface has no
             timeline content. The view itself owns the wordmark +
             accent + context block + sessions preview chrome; we
             pass the captain-facing signals as props and wire the
             restore action back through the emit. -->
        <IdleScreen
          v-if="timelineBlocks.length === 0"
          :profile="selectedProfile"
          :agent="sessionInfo.agent ?? activeProfile?.agent"
          :model="sessionInfo.model ?? activeProfile?.model"
          :cwd="idleCwd"
          :sessions="sessionListPreview"
          :total-session-count="sessionsForCwd.length"
          @restore-session="onRestoreSessionClick"
        />

        <Turn v-for="(block, blockIdx) in timelineBlocks" :key="block.groupKey" :role="block.role" :live="blockIdx === liveBlockIdx" :elapsed="elapsedFor(block.turnId)">
          <!-- Single thinking row per turn — same chrome regardless of
               whether prose accumulated. With text, the row is
               collapsable and reveals the reasoning trace; without
               text (claude-code-acp's empty extended-thinking chunks),
               the row stays a static "thought · 13s" badge so the
               captain still sees the agent IS reasoning. StreamCard
               drops the chevron + click affordance when there's no
               body to expand into. -->
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
          </template>

          <ToolChips v-if="block.toolCalls.length > 0" :views="block.toolCalls.map((t) => format(t.call, adapterFor(t.call.agentId)))" grouped />

          <!-- Inline terminal cards: one per tool call carrying a terminal id. -->
          <!-- Reads live stdout / stderr / exit through useTerminals().byId(). -->
          <template v-for="entry in block.toolCalls" :key="`term-${entry.call.toolCallId}`">
            <TerminalCard v-if="terminalIdForCall(entry.call)" :terminal-id="terminalIdForCall(entry.call) ?? ''" :instance-id="activeInstanceId" @cancel="onCancel" />
          </template>

          <template v-for="entry in block.turnEntries" :key="`turn-${entry.createdAt}`">
            <ChatBody v-if="entry.turn.role === TurnRole.Agent" :role="Role.Assistant" :text="entry.turn.text" markdown />
            <template v-else>
              <ChatBody :role="Role.User">{{ entry.turn.text }}</ChatBody>
              <Attachments v-if="entry.turn.attachments && entry.turn.attachments.length > 0" :attachments="entry.turn.attachments" @open="onAttachmentOpen" />
            </template>
          </template>
        </Turn>
      </div>
    </div>

    <PermissionStack :views="permissionRowQueue" @reply="(requestId, optionId) => onPermissionReply(requestId, optionId)" />

    <template #composer>
      <QueueStrip :messages="queueRows" @edit="onQueueEdit" @send="onQueueSend" @drop="onQueueDrop" @drop-all="onQueueDropAll" />
      <Composer ref="composerRef" :sending="sending" :can-cancel="phase !== Phase.Idle" @submit="onSubmit" @cancel="onCancel" />
    </template>
  </Frame>

  <!-- Modal-class permission UI — driven by `view.call.permissionUi
       === Modal` from the formatter. Today only `plan-exit` declares
       Modal; future heavy-confirm flows opt in by setting the same
       discriminator. Top-level so the backdrop covers the viewport
       regardless of any chat-transcript scroll position. -->
  <PermissionModal
    v-if="activeModalView"
    :view="activeModalView"
    @reply="(optionId) => onPermissionReply(activeModalView!.request.requestId, optionId)"
  />

  <!-- Rename-instance modal — singleton driven by
       `useRenameInstanceModal()`. Body composes `<ModalDescription>`
       above `<ModalInput>` per the compose-not-bag pattern; the
       modal chrome stays generic. -->
  <Modal
    v-if="renameModal.target.value"
    :title="`rename · ${renameModal.target.value.currentName ?? renameModal.target.value.instanceId.slice(0, 8)}`"
    :tone="ToastTone.Warn"
    :icon="faPenToSquare"
    :dismissable="true"
    @dismiss="onRenameCancel"
  >
    <template #actions>
      <Button :tone="ButtonTone.Neutral" @click="onRenameCancel">cancel</Button>
      <Button :tone="ButtonTone.Ok" :variant="ButtonVariant.Solid" @click="onRenameAccept">save</Button>
    </template>
    <ModalDescription> Lowercase letters, digits, <code>_</code>, <code>-</code>. Up to 16 chars. Empty clears the name. </ModalDescription>
    <ModalInput v-model:value="renameDraft" placeholder="ask, plan, review…" :validate="validateInstanceName" @submit="onRenameAccept" />
  </Modal>

  <CommandPalette />
</template>

<style scoped>
@reference '../assets/styles.css';

/* No `gap` between turns — each turn's role-color left border runs
 * the full height of its `.turn` element, so abutting turns produce
 * one continuous color stripe that switches color at the role
 * boundary (captain green ↔ pilot red). Visual breathing between
 * turns comes from each turn's own `py-1` instead. */
.chat-transcript {
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
  /* Positioning context for the scoped <Loading> + Modal overlays.
   * The wrapper itself stays padding-free so a `position: absolute;
   * inset: 0` cover paints edge-to-edge — the gutter padding lives
   * on `.chat-transcript-inner`. Without this split the cover stops
   * at the padding edge and leaves visible slivers of half-rendered
   * chat peeking through during session restore. */
  position: relative;
}

.chat-transcript-inner {
  @apply flex min-h-0 flex-1 flex-col;
  padding: 0 14px 0 4px;
}

/* idle screen — centered wordmark + LFG accent + kbd legend +
 * live-sessions table. Renders only when no timeline blocks exist. */
</style>
