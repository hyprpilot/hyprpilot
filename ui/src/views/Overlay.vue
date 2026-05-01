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
import { faListCheck } from '@fortawesome/free-solid-svg-icons'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'

import {
  type BreadcrumbCount,
  Button,
  ButtonTone,
  ButtonVariant,
  type ComposerPill,
  ComposerPillKind,
  Loading,
  MarkdownBody,
  Modal,
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
  isEditableTarget,
  pushToast,
  pushToQueue,
  removeFromQueue,
  startActiveInstance,
  useStickToBottom,
  startQueueDispatcher,
  stopQueueDispatcher,
  StreamItemKind,
  truncateCwd,
  TurnRole,
  useActiveInstance,
  useAdapter,
  resetPermissions,
  useAttachments,
  useHomeDir,
  useKeymap,
  useKeymaps,
  usePalette,
  usePermissions,
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
  type PlanEntry
} from '@composables'
import { type Attachment, invoke, Modifier, TauriCommand } from '@ipc'
import { formatToolCall, log } from '@lib'
import { Attachments, Body as ChatBody, ChangeBanner, StreamCard, TerminalCard, ToolChips, Turn } from '@views/chat'
import { Composer, PermissionStack, QueueStrip } from '@views/composer'
import { Frame } from '@views/header'
import { CommandPalette, isPaletteLeafId, openRootLeaf, openRootPalette, PaletteLeafId } from '@views/palette'

const { submit, cancel } = useAdapter()
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
const sessionListPreview = computed(() => sessionList.value.slice(0, IDLE_SESSIONS_PREVIEW))

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
const { pending: permissionPrompts, allow, deny } = usePermissions()
const { openTurnId } = useTurns()
const { info: sessionInfo } = useSessionInfo()
const { homeDir } = useHomeDir()
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

const headerCwd = computed(() => {
  const raw = sessionInfo.value.cwd
  if (!raw) {
    return undefined
  }

  return truncateCwd(raw, 32, homeDir.value)
})

const headerCounts = computed<BreadcrumbCount[]>(() => [
  { id: PaletteLeafId.Mcps, label: 'mcps', count: sessionInfo.value.mcpsCount },
  { id: PaletteLeafId.Instances, label: 'instances', count: instancesCount.value },
  { id: PaletteLeafId.Sessions, label: 'sessions', count: sessionList.value.length }
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
    openRootLeaf(id, { mcps: { instanceId, agentLabel: activeAgentId.value ?? 'agent' } })

    return
  }
  openRootLeaf(id)
}

function onToggleCwd(): void {
  openRootLeaf(PaletteLeafId.Cwd)
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
  log.info('keybind invoked', { action: 'toggle', target: 'window', via: 'capture' })
  pushToast(ToastTone.Ok, 'ctrl+q fired — invoking window_toggle')
  void invoke(TauriCommand.WindowToggle)
    .then((visible) => {
      log.info('window_toggle ok', { visible: String(visible) })
    })
    .catch((err: unknown) => {
      log.warn('window_toggle failed', { err: String(err) })
      pushToast(ToastTone.Err, `window_toggle failed: ${String(err)}`)
    })
}

onMounted(async () => {
  window.addEventListener('keydown', windowToggleCaptureListener, { capture: true })
  startQueueDispatcher()
  try {
    stopActiveInstanceStore = await startActiveInstance()
  } catch (err) {
    log.error('invoke failed', { command: 'startActiveInstance' }, err)
    pushToast(ToastTone.Err, `active-instance bind failed: ${String(err)}`)
  }
  try {
    stopStream = await startSessionStream()
  } catch (err) {
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
  const candidate = raw['terminal_id'] ?? raw['terminalId']

  return typeof candidate === 'string' && candidate.length > 0 ? candidate : undefined
}

// claude-code-acp serializes thinking as a `tool_call` with kind:
// "think" rather than as `agent_thought_chunk` session-update — so
// the thought body lives on `content[].text` (the tool-call text
// blocks) plus the chip's `title` as a one-line summary. Stitch
// them: title leads, content paragraphs follow.
function thoughtText(call: {
  title?: string
  content: { type?: string; text?: string }[]
  rawInput?: Record<string, unknown>
}): string {
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
    const raw = call.rawInput['thought'] ?? call.rawInput['text'] ?? call.rawInput['description']
    if (typeof raw === 'string') {
      parts.push(raw)
    }
  }

  return parts.join('\n\n')
}

async function onAttachmentOpen(att: Attachment): Promise<void> {
  if (!att.path) {
    return
  }
  try {
    const { open } = await import('@tauri-apps/plugin-shell')
    await open(att.path)
  } catch (err) {
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
  } catch (err) {
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

/// Plan-modal trigger: a small allowlist of plan / mode-switch tool
/// names PLUS a payload-shape fallback for vendors that ship a
/// markdown body without identifying the tool. Tool-name match is
/// case-insensitive AND collapses whitespace + non-letters
/// (`Switch mode` / `switch_mode` / `SwitchMode` all match).
const MARKDOWN_MODAL_TOOLS = new Set(['switchmode', 'exitplanmode', 'plan', 'planmode'])

function normalizeToolName(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]/g, '')
}

function pickMarkdownBody(raw: Record<string, unknown> | undefined): string | undefined {
  if (!raw) {
    return undefined
  }
  for (const key of ['plan', 'document'] as const) {
    const v = raw[key]
    if (typeof v === 'string' && v.trim().length > 0) {
      return v
    }
  }
  return undefined
}

interface MarkdownModalView {
  requestId: string
  tool: string
  body: string
}

function modalBodyOf(p: {
  tool: string
  rawInput?: Record<string, unknown>
  contentText?: string
}): string | undefined {
  // Tool-name allowlist routes the prompt to the modal regardless of
  // payload shape; rawInput.plan / rawInput.document wins, otherwise
  // contentText, otherwise a synthetic placeholder so the modal
  // still surfaces (the captain needs to accept / reject the mode
  // change even when the plan body doesn't ride along).
  if (MARKDOWN_MODAL_TOOLS.has(normalizeToolName(p.tool))) {
    const fromRaw = pickMarkdownBody(p.rawInput)
    if (fromRaw) {
      return fromRaw
    }
    if (typeof p.contentText === 'string' && p.contentText.trim().length > 0) {
      return p.contentText
    }
    return '_no plan body supplied_'
  }
  // Tools outside the allowlist: only route to modal when rawInput
  // explicitly carries a plan-shape key. Tools with content blocks
  // (descriptions, captions) stay in the inline permission stack.
  return pickMarkdownBody(p.rawInput)
}

const markdownModalPrompt = computed<MarkdownModalView | undefined>(() => {
  for (const p of permissionPrompts.value) {
    const body = modalBodyOf(p)
    if (body) {
      return { requestId: p.requestId, tool: p.tool, body }
    }
  }
  return undefined
})

const standardPermissionPrompts = computed(() =>
  permissionPrompts.value.filter((p) => modalBodyOf(p) === undefined)
)

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
    profileId: selectedProfile.value
  })

  const instanceId = activeInstanceId.value ?? mintInstanceId()
  useActiveInstance().set(instanceId)

  // Submit-while-busy: queue instead of dispatching. The composer
  // clears as in the dispatch path so the user can keep typing; the
  // K-260 turn-end watcher in `useQueue` drains the head when the
  // in-flight turn lands `acp:turn-ended` with stop_reason=end_turn.
  if (phase.value !== Phase.Idle) {
    pushToQueue(instanceId, { text, pills: attachments, skillAttachments })
    composerRef.value?.clear()
    clearAttachments()

    return
  }

  sending.value = true
  // The user turn lands as a daemon-emitted `TranscriptItem::UserPrompt`
  // event; the demuxer in `use-session-stream` routes it through to
  // `pushTranscriptChunk`. No optimistic mirror here — daemon is the
  // single source of truth.
  submit({ text, instanceId, profileId: selectedProfile.value, attachments: wireAttachments })
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
</script>

<template>
  <Frame
    :profile="selectedProfile ?? sessionInfo.agent ?? 'none'"
    :phase="phase"
    :mode-tag="sessionInfo.mode"
    :provider="sessionInfo.agent ?? activeProfile?.agent"
    :model="sessionInfo.model ?? activeProfile?.model"
    :title="sessionInfo.title"
    :cwd="headerCwd"
    :counts="headerCounts"
    :git-status="sessionInfo.gitStatus"
    @pill-click="onPillClick"
    @breadcrumb-click="onBreadcrumbClick"
    @toggle-cwd="onToggleCwd"
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
        <!-- idle screen: empty composer, no live blocks. Centered
           wordmark + "LFG." accent + kbd legend + live-sessions
           mini-table. Triggers the moment the chat surface has no
           timeline content. -->
        <section v-if="timelineBlocks.length === 0" class="idle-screen" data-testid="idle-screen">
          <div class="idle-wordmark">hyprpilot</div>
          <div class="idle-accent">LFG.</div>
          <div class="idle-kbd-legend">
            <span class="idle-kbd">Ctrl+K</span><span class="idle-kbd-label">command palette</span> <span class="idle-kbd">@</span
            ><span class="idle-kbd-label">reference a file or folder</span> <span class="idle-kbd">+</span><span class="idle-kbd-label">attach a skill or reference</span>
            <span class="idle-kbd">Esc</span><span class="idle-kbd-label">close overlay</span>
          </div>
          <div v-if="sessionList.length > 0" class="idle-sessions">
            <header class="idle-sessions-header">
              <span class="idle-sessions-count">{{ sessionList.length }}</span>
              <span class="idle-sessions-title">sessions</span>
              <span class="idle-sessions-line" />
            </header>
            <div class="idle-sessions-headrow">
              <span />
              <span>title</span>
              <span>cwd</span>
              <span>doing</span>
            </div>
            <div
              v-for="s in sessionListPreview"
              :key="s.sessionId"
              class="idle-sessions-row"
              :role="s.sessionId ? 'button' : undefined"
              :tabindex="s.sessionId ? 0 : undefined"
              :aria-label="s.sessionId ? `restore session ${s.title || s.sessionId}` : undefined"
              :data-restorable="Boolean(s.sessionId)"
              @click="onRestoreSessionClick(s.sessionId)"
              @keydown.enter.prevent="onRestoreSessionClick(s.sessionId)"
              @keydown.space.prevent="onRestoreSessionClick(s.sessionId)"
            >
              <span class="idle-sessions-dot" aria-hidden="true">○</span>
              <span class="idle-sessions-cell">{{ s.title || s.sessionId }}</span>
              <span class="idle-sessions-cell idle-sessions-cwd">{{ s.cwd }}</span>
              <span class="idle-sessions-cell idle-sessions-doing">—</span>
            </div>
            <div v-if="sessionList.length > sessionListPreview.length" class="idle-sessions-more">
              +{{ sessionList.length - sessionListPreview.length }} more — Ctrl+K → sessions
            </div>
          </div>
        </section>

        <Turn v-for="(block, blockIdx) in timelineBlocks" :key="block.groupKey" :role="block.role" :live="blockIdx === liveBlockIdx">
          <template v-for="entry in block.thoughts" :key="`thought-${entry.call.toolCallId}`">
            <StreamCard
              :kind="StreamKind.Thinking"
              :active="blockIdx === liveBlockIdx"
              label="thought"
              :text="thoughtText(entry.call)"
            />
          </template>
          <template v-for="entry in block.streamEntries" :key="`stream-${entry.createdAt}`">
            <StreamCard
              v-if="entry.item.kind === StreamItemKind.Thought"
              :kind="StreamKind.Thinking"
              :active="blockIdx === liveBlockIdx"
              label="thought"
              :text="entry.item.text"
            />
            <StreamCard
              v-else-if="entry.item.kind === StreamItemKind.Plan"
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
          </template>

          <!-- provider passed `undefined` for now: resolves to baseRegistry. Plumb -->
          <!-- `activeProfile?.agent` → `profiles_list`'s vendor once per-adapter overrides land. -->
          <ToolChips v-if="block.toolCalls.length > 0" :items="block.toolCalls.map((t) => formatToolCall(t.call))" grouped />

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

    <PermissionStack :prompts="standardPermissionPrompts" @allow="onAllow" @deny="onDeny" />

    <template #composer>
      <QueueStrip :messages="queueRows" @drop="onQueueDrop" @drop-all="onQueueDropAll" />
      <Composer ref="composerRef" :sending="sending" :can-cancel="phase !== Phase.Idle" @submit="onSubmit" @cancel="onCancel" />
    </template>
  </Frame>

  <!-- Plan-modal — markdown body for permissions matching the
       allowlist (`switch_mode` / `exit_plan_mode` / `plan` /
       `plan_mode`). Top-level so the `position: fixed` backdrop
       covers the viewport regardless of any chat-transcript scroll
       position or stacking context inside the Frame. -->
  <Modal
    v-if="markdownModalPrompt"
    :title="`plan · ${markdownModalPrompt.tool}`"
    :tone="ToastTone.Warn"
    :icon="faListCheck"
    :dismissable="false"
  >
    <template #actions>
      <Button :tone="ButtonTone.Err" @click="onDeny(markdownModalPrompt.requestId)">reject</Button>
      <Button :tone="ButtonTone.Ok" :variant="ButtonVariant.Solid" @click="onAllow(markdownModalPrompt.requestId)">accept</Button>
    </template>
    <MarkdownBody :source="markdownModalPrompt.body" />
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
.idle-screen {
  @apply flex flex-col items-center justify-center;
  flex: 1 1 auto;
  min-height: 100%;
  padding: 24px;
  color: var(--theme-fg-dim);
}

.idle-wordmark {
  font-family: var(--theme-font-mono);
  font-size: 26px;
  font-weight: 500;
  letter-spacing: -0.3px;
  color: var(--theme-fg);
}

.idle-accent {
  margin-top: 4px;
  font-family: var(--theme-font-mono);
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 1px;
  color: var(--theme-accent);
}

.idle-kbd-legend {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 4px 10px;
  margin-top: 22px;
  font-family: var(--theme-font-mono);
  font-size: 0.62rem;
}

.idle-kbd {
  color: var(--theme-accent);
}

.idle-kbd-label {
  color: var(--theme-fg-dim);
}

.idle-sessions {
  width: 100%;
  max-width: 640px;
  margin-top: 26px;
}

.idle-sessions-header {
  @apply flex items-center;
  margin-bottom: 6px;
  font-family: var(--theme-font-mono);
  font-size: 0.62rem;
  color: var(--theme-fg-ink-2);
  gap: 6px;
}

.idle-sessions-count {
  color: var(--theme-accent);
  font-weight: 700;
}

.idle-sessions-title {
  text-transform: lowercase;
}

.idle-sessions-line {
  flex: 1;
  height: 1px;
  background-color: var(--theme-border);
  margin-left: 8px;
}

.idle-sessions-headrow {
  display: grid;
  grid-template-columns: 14px 1fr 170px 110px;
  column-gap: 12px;
  padding: 4px 10px;
  font-family: var(--theme-font-mono);
  font-size: 0.56rem;
  color: var(--theme-fg-dim);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  border-bottom: 1px solid var(--theme-border);
}

.idle-sessions-row {
  display: grid;
  grid-template-columns: 14px 1fr 170px 110px;
  column-gap: 12px;
  align-items: center;
  padding: 7px 10px;
  border-bottom: 1px solid var(--theme-border);
  border-left: 2px solid var(--theme-status-ok);
  background-color: var(--theme-surface);
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
  color: var(--theme-fg);
  transition: background-color 0.12s ease-out;
}

.idle-sessions-row[data-restorable='true'] {
  cursor: pointer;
}

.idle-sessions-row[data-restorable='true']:hover,
.idle-sessions-row[data-restorable='true']:focus-visible {
  background-color: var(--theme-surface-alt);
  outline: 0;
}

.idle-sessions-dot {
  color: var(--theme-status-ok);
}

.idle-sessions-more {
  padding: 6px 10px;
  font-family: var(--theme-font-mono);
  font-size: 0.62rem;
  color: var(--theme-fg-dim);
  border-top: 1px solid var(--theme-border-soft);
  background-color: var(--theme-surface);
  letter-spacing: 0.4px;
}

.idle-sessions-cell {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--theme-fg);
}

.idle-sessions-cwd {
  color: var(--theme-fg-ink-2);
}

.idle-sessions-doing {
  color: var(--theme-status-ok);
}
</style>
