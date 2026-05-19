<script setup lang="ts">
/**
 * Overlay shell — the single page-level view the Tauri webview mounts
 * (see `App.vue`). Composes the chat primitives into the running app.
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
 *   useSessionHistory   → warms the session store for the palette
 *   useTranscript       → user/assistant turns
 *   useStream           → thought / plan stream items
 *   useTools            → tool-call records for the inline chip row
 *   useActiveInstance   → current instance id for the transcript data-attr
 *   startSessionStream  → starts the demuxed Tauri event pump
 */
import { faPenToSquare } from '@fortawesome/free-solid-svg-icons'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'

import {
  type BreadcrumbCount,
  Button,
  ButtonTone,
  ButtonVariant,
  type ComposerPill,
  ComposerPillKind,
  Modal,
  ModalDescription,
  ModalInput,
  Phase,
  type QueuedMessage,
  RemotePairModal,
  Toast,
  ToastTone
} from '@components'
import {
  pushToast,
  startActiveInstance,
  useActiveInstance,
  useAdapter,
  resetPermissions,
  useAttachments,
  useComposer,
  useDaemonCwd,
  useFocusPrefetch,
  useKeymap,
  useKeymaps,
  useNotifications,
  usePalette,
  usePermissions,
  useRenameInstanceModal,
  usePhase,
  useProfiles,
  useQueue,
  useRemotePair,
  useSessionHistory,
  useSessionInfo,
  useToasts,
  type KeymapEntry,
  startRemotePairListener,
  type InstanceId
} from '@composables'
import { type Attachment, invoke, Modifier, TauriCommand } from '@ipc'
import { isRemoteHost, subscribePair } from '@ipc/remote-bridge'
import { log } from '@lib'
import { PermissionModal, Viewport as ChatViewport } from '@views/chat'
import { Composer, PermissionStack, QueueStrip } from '@views/composer'
import { Frame } from '@views/header'
import { IdleScreen } from '@views/idle'
import { CommandPalette, commitInstanceRename, isPaletteLeafId, openRootLeaf, openRootPalette, PaletteLeafId, validateInstanceName } from '@views/palette'

const { submit, cancel } = useAdapter()
const { pending: pendingAttachments, clear: clearAttachments } = useAttachments()
const { phase } = usePhase()
const { profiles, selected: selectedProfile } = useProfiles()
const { info: sessionInfo } = useSessionInfo()
// Session history is wired but the overlay shell doesn't surface a
// session picker directly — the palette view owns that UX. Keeping
// the binding live so the backend stays warm; the list count rides
// on the row-2 sessions breadcrumb pill.
//
// Prefer the focused instance's spawning profile over the picker's
// value: header chrome reads `sessionInfo.profileId` for the profile
// pill, so the session list has to track the same axis to stay
// consistent. Without this, switching focus between instances with
// different profiles flips the header but leaves the session list on
// whatever profile the picker last touched. Falls back to the picker
// when no instance is focused yet (idle screen).
const sessionListProfileId = computed(() => sessionInfo.value.profileId ?? selectedProfile.value)
const sessionListAgentId = computed(() => {
  const profileId = sessionListProfileId.value

  return profileId ? profiles.value.find((p) => p.id === profileId)?.agent : undefined
})
const { sessions: sessionList, load: restoreSession } = useSessionHistory(sessionListAgentId, sessionListProfileId)

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
function onRestoreSessionClick(sessionId: string | undefined, cwd: string): void {
  if (!sessionId) {
    return
  }
  void restoreSession(sessionId, cwd)
}

const { id: activeInstanceId, count: instancesCount } = useActiveInstance()
const { count: notificationsCount } = useNotifications()
// Phase C1: chat-body state (timeline blocks, virtualization,
// stick-to-bottom) lives inside `<ChatViewport>`. Overlay reads only
// the cross-feature stores that drive surfaces other than the body.
const { rowQueue: permissionRowQueue, modalQueue: permissionModalQueue, respond: respondPermission } = usePermissions()
const { daemonCwd } = useDaemonCwd()
const { items: queuedItems, remove: removeFromActiveQueue, dispatch: dispatchActiveQueue, edit: editActiveQueueItem, clear: flushActiveQueue } = useQueue()
const { entries: toastEntries, dismiss: dismissToast } = useToasts()
const activeToast = computed(() => toastEntries.value[0])

const queueRows = computed<QueuedMessage[]>(() => queuedItems.value.map((q) => ({ id: q.id, text: q.text })))

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
 * Both wire fields ship pre-formatted display strings (`~/proj/foo`)
 * — the daemon collapses `$HOME` server-side via
 * `tools::path::display_cwd`. The UI renders verbatim; chrome's CSS
 * `text-overflow: ellipsis` handles overflow.
 */
const cwdDisplay = computed<string | undefined>(() => sessionInfo.value.cwd ?? daemonCwd.value)
const headerCwd = cwdDisplay
const headerCwdFull = cwdDisplay
const idleCwd = cwdDisplay

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

// Phase C1: Block grouping, live-block index, elapsed / thinking
// elapsed labels, and `useNow` ticker live inside `<ChatViewport>`
// (the virtualized chat-body component). Overlay no longer assembles
// the timeline — the body reads off the daemon snapshot directly via
// `useChatViewport` and projects pages through `timelineBlocksFromSnapshot`.

function firePermission(action: 'allow' | 'deny'): void {
  // TODO: Tab = next row cycling. Today the approval keybind always
  // addresses the oldest-active (first non-queued) prompt.
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
      action,
      target: 'permission',
      reason: 'no_basic_variant',
      offered: active.options.map((o) => o.kind)
    })
    pushToast(ToastTone.Warn, `${action} keybind: agent didn't offer ${targetKind}; click an option directly`)

    return
  }
  log.info('keybind invoked', {
    action,
    target: 'permission',
    optionId: opt.optionId,
    kind: opt.kind
  })
  void onPermissionReply(active.request.requestId, opt.optionId)
}

const { keymaps } = useKeymaps()
const { closeAll: closeAllPalettes, isOpen: paletteIsOpen, focusInput: focusPaletteInput } = usePalette()
const composer = useComposer()

// Remote-bridge pair-confirm modal. Auto-opens whenever the daemon
// emits `remote:pair-request` (a phone / browser hit `wss://…/ws`).
// Captain types the 4-word code shown on the connecting device → the
// pending WS upgrades to authenticated. No tokens persist.
const { active: remotePairActive } = useRemotePair()
let stopRemotePairListener: (() => void) | undefined

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
        // Refocus whichever input is "primary" right now: the
        // palette's search input when a palette is open, otherwise
        // the composer textarea. Lets the captain Ctrl+F back into
        // typing after a click pulled focus elsewhere (a tool pill,
        // a permission button, the transcript scroll area).
        binding: keymaps.value.chat.focus_input,
        handler: () => {
          if (paletteIsOpen()) {
            focusPaletteInput()
          } else {
            composer.focus()
          }

          return true
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
          void dispatchActiveQueue()

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
          void removeFromActiveQueue(head.id)

          return true
        }
      }
    ]
  }
)

let stopActiveInstanceStore: (() => void) | undefined
let stopFocusPrefetch: (() => void) | undefined
let unsubscribePairForBrimSync: (() => void) | undefined

// `useFocusPrefetch` must be invoked during setup so its
// `useQueryClient()` injection lookup hits the active component
// context. The actual brim-sync + listener registration runs inside
// `onMounted` below — instantiating here only resolves the client
// reference, no IPC calls fire yet.
const focusPrefetch = useFocusPrefetch()

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

  try {
    stopActiveInstanceStore = await startActiveInstance()
  } catch(err) {
    log.error('invoke failed', { command: 'startActiveInstance' }, err)
    pushToast(ToastTone.Err, `active-instance bind failed: ${String(err)}`)
  }

  // `startSessionStream` is hoisted to `main.ts` so its listeners
  // wire BEFORE Overlay mounts (and BEFORE the boot snapshot fires).
  // Used to live here; moving it closed a remote-bridge race where
  // events arriving between pair-auth and Overlay mount got dropped.
  // The re-entry guard in `startSessionStream` makes a stray call
  // safe but unnecessary.

  try {
    stopRemotePairListener = await startRemotePairListener()
  } catch(err) {
    log.warn('remote: pair listener failed to mount', { err: String(err) })
  }

  // Brim-sync + per-focus prefetch (Phase C2). The focus-prefetch
  // listener registers immediately; the brim-sync runs once the
  // bridge is authenticated. Desktop is always authenticated at
  // mount; remote waits on the pair handshake.
  try {
    stopFocusPrefetch = await focusPrefetch.start()
  } catch(err) {
    log.warn('focus-prefetch: start failed', { err: String(err) })
  }
  const runBrimSync = (): void => {
    void focusPrefetch.brimSync(activeInstanceId.value).catch((err: unknown) => {
      log.warn('brim-sync failed', { err: String(err) })
    })
  }

  if (!isRemoteHost()) {
    runBrimSync()
  } else {
    let fired = false

    unsubscribePairForBrimSync = subscribePair((view) => {
      if (fired || !view.authenticated) {
        return
      }
      fired = true
      unsubscribePairForBrimSync?.()
      unsubscribePairForBrimSync = undefined
      runBrimSync()
    })
  }
})

onUnmounted(() => {
  window.removeEventListener('keydown', windowToggleCaptureListener, { capture: true })
  stopActiveInstanceStore?.()
  stopActiveInstanceStore = undefined
  stopRemotePairListener?.()
  stopRemotePairListener = undefined
  stopFocusPrefetch?.()
  stopFocusPrefetch = undefined
  unsubscribePairForBrimSync?.()
  unsubscribePairForBrimSync = undefined
})

// Skill attachments are per-turn but tied to the active instance —
// switching to another instance mid-compose discards any pending
// picks (they were assembled against the previous instance's context).
watch(activeInstanceId, (next, prev) => {
  if (prev && next !== prev) {
    clearAttachments()
  }
})

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

/// One modal at a time — permission UI is blocking by nature, so
/// stacking doesn't add value. Subsequent modal-class prompts wait
/// behind this one in `permissionModalQueue`.
const activeModalView = computed(() => permissionModalQueue.value[0])

async function onPermissionReply(requestId: string, optionId: string, feedback?: string): Promise<void> {
  log.info('permission click', {
    requestId,
    optionId,
    hasFeedback: feedback !== undefined && feedback.trim().length > 0
  })

  try {
    await respondPermission(requestId, optionId, feedback)
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

/**
 * Mime types the daemon's `attachment_to_block` encoder routes
 * through `TextResourceContents` (reads `att.body`, ignores
 * `att.data`). Keep this in lockstep with
 * `Composer.vue::isTextMime` and `acp/instance.rs::mime_category`.
 */
function isTextMime(mime: string | undefined): boolean {
  if (!mime) {
    return false
  }

  if (mime.startsWith('text/')) {
    return true
  }

  return mime === 'application/json' || mime === 'application/xml' || mime === 'application/x-yaml' || mime === 'application/toml'
}

/**
 * Project attachment-kind composer pills (paperclip / drag-drop /
 * Ctrl+P) onto the wire `Attachment` shape so the daemon's
 * `build_prompt_blocks` can emit the right ACP `ContentBlock`
 * variant. Skill-style pills (palette-pushed onto `useAttachments`)
 * skip this path — they already arrive as wire `Attachment`s.
 *
 * The pill's `data` field carries either base64 binary OR plain
 * text depending on the mime category; this projector mirrors that
 * dispatch onto the wire `body` / `data` axis the daemon expects.
 *
 * `path` is synthesized from the optional `fileName` (drag-drop /
 * file picker) or the MIME's extension (clipboard paste) so the
 * Rust side's `mime_guess` fallback still works on the extension
 * if the explicit `mime` field is ever stripped en route.
 */
function pillsToAttachments(pills: ComposerPill[]): Attachment[] {
  return pills
    .filter((p) => p.kind === ComposerPillKind.Attachment)
    .map((p) => {
      const mime = p.mimeType ?? 'application/octet-stream'
      const ext = mime.split('/')[1]?.split('+')[0] ?? 'bin'
      const synthName = p.fileName && p.fileName.length > 0 ? p.fileName : `${p.id}.${ext}`
      const text = isTextMime(p.mimeType)

      return {
        slug: p.id,
        path: synthName,
        // Text-shaped attachments ride on `body` (the daemon's
        // `TextResourceContents` encoder reads it); binary on
        // `data` (Image / Audio / Blob encoders read base64 there).
        body: text ? p.data : '',
        title: p.label,
        data: text ? undefined : p.data,
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
const editingQueueSlot = ref<{ instanceId: InstanceId; itemId: string } | undefined>(undefined)

function onSubmit(payload: { text: string; attachments: ComposerPill[] }): void {
  const { text, attachments } = payload
  // Skill / resource attachments live in the `useAttachments` singleton
  // (the skills palette pushes onto it). They snapshot at submit time
  // so a resubmit after cancel sends the same set; submit-ack clears.
  const skillAttachments = [...pendingAttachments.value]
  // Attachment pills (paperclip / drag-drop / Ctrl+P) project onto
  // the wire `Attachment` shape; the daemon's `build_prompt_blocks`
  // dispatches to ACP `ContentBlock::Image` / `Audio` / `Resource`
  // based on the mime category (versus skill resources which arrive
  // pre-resolved as wire `Attachment`s).
  const fileAttachments = pillsToAttachments(attachments)
  const wireAttachments = [...skillAttachments, ...fileAttachments]

  log.info('composer submit', {
    text_len: text.length,
    file_attachments: fileAttachments.length,
    skill_attachments: skillAttachments.length,
    profileId: selectedProfile.value,
    editing: editingQueueSlot.value !== undefined
  })

  // **Daemon is the source of truth for instance identity.** Earlier
  // versions minted the UUID UI-side and shipped it via
  // `session_submit { instanceId }`; the daemon's adopt-verbatim
  // fallback was permissive, but that meant the UI's chrome
  // (ChatViewport `:key`, `useActiveInstance.set`, the InfiniteQuery
  // key) all flipped onto an id that didn't exist server-side until
  // the spawn task landed — every per-instance surface raced the
  // spawn and a "not found" snapshot error stranded the cache. The
  // right shape is: omit `instanceId` when there's nothing focused,
  // let the daemon mint via `InstanceKey::new_v4()`, and read the
  // freshly-issued id off the RPC reply (`SubmitResult.instanceId`).
  const instanceId = activeInstanceId.value

  // Edit-resubmit: captain pulled this entry into the composer via
  // the queue-strip edit button. Daemon owns the queue now — submit
  // becomes an in-place `queue/edit` RPC. The item stays at its
  // original slot with the new text + attachments. The composer's
  // existing draft has been replaced with the queued item's text in
  // `onQueueEdit`; clear it on a successful edit, restore the
  // pending state on rejection so the captain can retry.
  const editing = editingQueueSlot.value

  if (editing && editing.instanceId === instanceId) {
    void editActiveQueueItem(editing.itemId, text, wireAttachments)
      .then(() => {
        editingQueueSlot.value = undefined
        composerRef.value?.clear()
        clearAttachments()
      })
      .catch((err) => {
        log.error('queue/edit failed', { instanceId, itemId: editing.itemId }, err)
        pushToast(ToastTone.Err, `edit failed: ${String(err)}`)
        // Don't clear the composer — captain keeps the draft + can
        // retry. `editingQueueSlot` stays set so the next submit
        // attempts the edit again instead of falling through to a
        // fresh send.
      })

    return
  }

  // Submit always goes through `prompts/send` (server-side
  // `session_submit`). The daemon's auto-route decides:
  // idle → dispatch immediately; busy → enqueue at the tail. UI
  // doesn't pre-check phase — the daemon is the single decider so
  // every transport (Vue desktop, mobile WS, hyprpilot-nvim) agrees.

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
    .then((result) => {
      // Daemon-issued id (`InstanceKey::new_v4()` server-side, or the
      // adopt-verbatim path when the caller passed one). When this
      // was a fresh-spawn — `instanceId` was undefined, the daemon
      // just minted — pin the result onto `useActiveInstance` so
      // ChatViewport's `:key` flips, the snapshot fetch hits an
      // existing actor, and the transcript-patcher's cache check
      // succeeds when the first live frame lands. Setting unconditionally
      // would race the daemon-pushed `acp:instances-focused` event on
      // a follow-up submit; the `!activeInstanceId.value` gate keeps
      // the reply path advisory.
      if (result.instanceId && !activeInstanceId.value) {
        useActiveInstance().set(result.instanceId)
      }
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
  void removeFromActiveQueue(id)
}

function onQueueDropAll(): void {
  void flushActiveQueue()
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
  const target = queuedItems.value.find((q) => q.id === itemId)

  if (!target) {
    return
  }
  // In-place edit: leave the queue item alone, pre-fill the composer
  // with its text + attachments, and pin the id so the next submit
  // routes through `queue/edit` instead of re-enqueueing. Branch on
  // mime type when projecting back to `ComposerPill`s — attachments
  // with a `mime` set on the wire shape are file uploads (paperclip
  // / drag-drop), so they need the Attachment pill shape so
  // `pillsToAttachments` re-finds them on submit; skill attachments
  // (slug + body, no mime) land as Resource pills.
  editingQueueSlot.value = { instanceId, itemId }
  composerRef.value?.setDraft({
    text: target.text,
    pills: (target.attachments ?? []).map((a) => {
      // File upload (paperclip / drag-drop) — has a real `mime`
      // type from the file picker. Skill / palette-pushed
      // attachments land without `mime`. Branch on presence; the
      // composer's pill shape carries `data` for binary OR text
      // (per `Composer.vue::isTextMime`) so we forward whichever
      // the wire shape had populated.
      if (a.mime) {
        const text = a.body && a.body.length > 0 ? a.body : ''

        return {
          kind: ComposerPillKind.Attachment,
          id: `attachment:${a.slug}`,
          label: a.title ?? a.slug,
          data: text.length > 0 ? text : (a.data ?? ''),
          mimeType: a.mime,
          fileName: a.title ?? a.path
        }
      }

      return {
        kind: ComposerPillKind.Resource,
        id: `attachment:${a.slug}`,
        label: a.title ?? a.slug,
        data: a.slug,
        mimeType: 'skill'
      }
    })
  })
  log.info('queue edit start', { instanceId, queuedItemId: itemId })
}

/**
 * Per-row "send now" — captain explicit dispatch for a specific
 * queued entry. Server pops + fires immediately; ACP serialises if
 * a turn is already in flight.
 */
function onQueueSend(itemId: string): void {
  log.info('queue send-now', { queuedItemId: itemId })
  void dispatchActiveQueue(itemId)
}
</script>

<template>
  <Frame
    :profile="sessionInfo.profileId ?? selectedProfile ?? sessionInfo.agent ?? 'none'"
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
    :notifications-count="notificationsCount"
    :git-status="sessionInfo.gitStatus"
    @pill-click="onPillClick"
    @breadcrumb-click="onBreadcrumbClick"
    @toggle-cwd="onToggleCwd"
    @close="onCloseOverlay"
    @instances-click="openRootLeaf(PaletteLeafId.Instances)"
    @notifications-click="openRootLeaf(PaletteLeafId.Notifications)"
    @palette-click="openRootPalette"
  >
    <template v-if="activeToast" #toast>
      <Toast :tone="activeToast.tone" :body="activeToast.body" @dismiss="dismissToast(activeToast.id)" />
    </template>

    <!--
      Chat transcript viewport (Phase C1 — virtualized over daemon
      snapshot pages). The component owns its own scroll element,
      `useStickToBottom`, infinite-query data, page-trim policy,
      live-event patches, and intersection-based load-more sentinel.
      Idle landing renders inside the `empty` slot so the empty-gate
      reads off the viewport's snapshot items, not the accumulator.
    -->
    <!-- `:key="activeInstanceId"` forces a clean remount on every
         instance flip. Every viewport-local concern (scroll position,
         `useStickToBottom.stuck`, `useChatViewport`'s listener IIFE +
         `pendingPatches` queue, `useSnapshotHydration`'s dedup sets)
         resets atomically. `useStickToBottom.onMounted` calls
         `scrollToBottom()` so the captain always lands at the latest
         message of the freshly-focused instance — no "loading
         earlier history" misfire because the new viewport hasn't
         scrolled yet (`hasUserScrolled` gate in `onScroll`). TanStack
         keeps the per-instance chat cache keyed by `instanceId` so
         content paints from cache the moment the new mount reads
         `query.data.value` — no IPC round-trip cost on the flip. -->
    <ChatViewport :key="activeInstanceId ?? 'idle'" :restoring="sessionInfo.restoring" @cancel="onCancel" @attachment-open="onAttachmentOpen">
      <template #empty>
        <IdleScreen
          :profile="selectedProfile"
          :agent="sessionInfo.agent ?? activeProfile?.agent"
          :model="sessionInfo.model ?? activeProfile?.model"
          :cwd="idleCwd"
          :sessions="sessionListPreview"
          :total-session-count="sessionsForCwd.length"
          @restore-session="onRestoreSessionClick"
          @open-palette="openRootPalette"
        />
      </template>
    </ChatViewport>

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
  <PermissionModal v-if="activeModalView" :view="activeModalView" @reply="(optionId, fb) => onPermissionReply(activeModalView!.request.requestId, optionId, fb)" />

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

  <!-- Remote-bridge pair confirm — auto-opens whenever a phone /
       browser hits the daemon's `wss://…/ws` and the daemon emits
       `remote:pair-request`. Captain types the 4-word code shown on
       the connecting device; on match the pending WS upgrades to
       authenticated. -->
  <RemotePairModal v-if="remotePairActive" :state="remotePairActive" />

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
  padding: 0 0.875rem 0 0.25rem;
}

/* idle screen — centered wordmark + LFG accent + kbd legend +
 * live-sessions table. Renders only when no timeline blocks exist. */
</style>
