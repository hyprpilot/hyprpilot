<script setup lang="ts">
/**
 * Composer row: pills (image attachments + skill attachments) +
 * autosizing textarea + send button. Owns compose text + image-pill
 * state via `useComposer`; reads skill attachments off the
 * `useAttachments` module-scope singleton (the skills palette pushes
 * there). The parent's `@submit` receives `{ text, attachments }`
 * — image pills go in the `attachments` slot, skill attachments
 * travel separately via `useAttachments().pending`.
 *
 * Ctrl+P (`composer.paste` binding) reads a clipboard image via
 * `tauri-plugin-clipboard-manager`'s `readImage()` (RGBA pixels +
 * dimensions) → encodes as PNG via canvas → base64 dataURL.
 *
 * Drag-and-drop: image files become attachment pills via the same
 * `FileReader` path; non-image files are ignored (skill attachments
 * are palette-driven, not drop-driven).
 */
import { faArrowTurnDown, faCircleNotch, faPaperclip, faStop } from '@fortawesome/free-solid-svg-icons'
import { readImage } from '@tauri-apps/plugin-clipboard-manager'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'

import CompletionPopover from './CompletionPopover.vue'
import ChatComposerPill from './ComposerPill.vue'
import { ToastTone, ComposerPillKind, type ComposerPill } from '@components'
import {
  type KeymapEntry,
  pushToast,
  useActiveInstance,
  useAttachments,
  useCompletion,
  useComposer,
  useDaemonCwd,
  useKeymap,
  useKeymaps,
  useSessionInfo,
  useTouchDevice
} from '@composables'
import { CompletionKind, invoke, Modifier, TauriCommand } from '@ipc'
import { blobToDataUrl, formatSize, getCaretCoordinates, log, rgbaToPngBlob } from '@lib'

const props = withDefaults(
  defineProps<{
    placeholder?: string
    disabled?: boolean
    sending?: boolean
    /**
     * Optional externally-supplied pills. When provided, the composer
     * renders these instead of its internal `useComposer` pill list —
     * lets parents (stories, palette pre-seeds) drive state without
     * re-owning the composable. The parent then listens on
     * `@removePill`.
     */
    pills?: ComposerPill[]
    /**
     * `true` when a turn is currently in flight on the active
     * instance. The composer renders a stop button stacked under the
     * send button while this is set; emits `@cancel` on click.
     * Parent decides what "in flight" means (typically `phase !==
     * Idle`).
     */
    canCancel?: boolean
  }>(),
  {
    placeholder: 'message pilot',
    disabled: false,
    sending: false,
    pills: undefined,
    canCancel: false
  }
)

const emit = defineEmits<{
  submit: [payload: { text: string; attachments: ComposerPill[] }]
  removePill: [id: string]
  cancel: []
}>()

const composer = useComposer()
const text = composer.text
const composerPills = composer.pills

const attachments = useAttachments()

// Touch-device probe — drives the Enter-key branch in `onEnter`. On a
// soft keyboard there's no Shift key, so the captain has no way to
// insert a newline if Enter submits. Coarse pointer → Enter becomes
// newline; the visible send button is the only submit path on mobile.
const touch = useTouchDevice()

// Skill attachments (palette-driven) render as resource pills
// alongside image attachment pills. The composer doesn't own the
// pending list — it only presents and forwards the remove intent.
const attachmentPills = computed<ComposerPill[]>(() =>
  attachments.pending.value.map((a) => ({
    kind: ComposerPillKind.Resource,
    id: `attachment:${a.slug}`,
    label: a.title ?? a.slug,
    data: a.slug,
    mimeType: 'skill'
  }))
)

const pillsToRender = computed<ComposerPill[]>(() => props.pills ?? [...attachmentPills.value, ...composerPills.value])

// Counter of in-flight FileReader / clipboard reads. While > 0 the
// composer renders an inline "loading attachment…" placeholder pill
// so the user gets immediate feedback on a click — large images can
// take ~500ms to base64-encode and would otherwise look like a
// dead button. Decrements when the read settles (success or error).
const attachmentLoading = ref(0)

const fileInputRef = ref<HTMLInputElement>()

const textareaRef = ref<HTMLTextAreaElement>()

function resize(): void {
  const el = textareaRef.value

  if (!el) {
    return
  }

  // When the composer is empty, leave the inline height unset so
  // CSS `min-height` is the sole governor of the rendered box.
  // Writing an inline `height: <scrollHeight>px` for empty content
  // freezes a value the layout engine resolved before the first
  // paint settled — on cold-overlay-popup in webkit2gtk this paints
  // at the rows="2" intrinsic (~65px) for one frame before
  // min-height pulls it back to 96px on the next reflow. Skipping
  // the inline write keeps the box at a consistent 96px floor from
  // the very first frame.
  if (el.value.length === 0) {
    el.style.height = ''

    return
  }
  el.style.height = 'auto'
  el.style.height = `${el.scrollHeight}px`
}

const completion = useCompletion()
const completionLeft = ref(0)
// When the popover would clip the viewport bottom, anchor from its
// own bottom edge instead of its top — this keeps a popover with
// fewer rows than the height estimate sitting flush against the
// caret line, rather than floating above with a gap. Exactly one of
// top / bottom is set per render; the other is `null`.
const completionTop = ref<number | null>(0)
const completionBottom = ref<number | null>(null)

const { keymaps } = useKeymaps()

useKeymap(textareaRef, (): KeymapEntry[] => {
  if (!keymaps.value) {
    return []
  }

  return [
    { binding: keymaps.value.chat.submit, handler: onEnter },
    { binding: keymaps.value.chat.newline, handler: () => false },
    { binding: keymaps.value.composer.paste, handler: onPasteImage },
    { binding: keymaps.value.composer.tab_completion, handler: onTab },
    { binding: keymaps.value.composer.shift_tab, handler: onTab },
    {
      // Force-open completion (manual ripgrep / chat-buffer scan).
      // Falls back to hardcoded Ctrl+Space when the wire-loaded
      // keymap predates the field. Same handler as Tab — when the
      // popover is already open, commits the active row.
      binding: keymaps.value.composer.completion ?? { modifiers: [Modifier.Ctrl], key: 'space' },
      handler: onTab
    },
    {
      binding: keymaps.value.composer.history_up,
      handler: onHistoryPrev,
      allowRepeat: true
    },
    {
      binding: keymaps.value.composer.history_down,
      handler: onHistoryNext,
      allowRepeat: true
    }
  ]
})

/**
 * Composer-level keystroke pre-filter for the completion popover.
 * Runs BEFORE the keymap dispatcher when the popover is open so
 * arrow / Enter / Esc / Tab route to the completion state machine
 * instead of the existing chat / history bindings. When the popover
 * is closed, this is a no-op and falls through to the keymap chain.
 */
function onTextareaKeydown(e: KeyboardEvent): void {
  if (!completion.state.value.open) {
    return
  }

  if (e.key === 'ArrowDown') {
    e.preventDefault()
    e.stopPropagation()
    completion.selectNext()

    return
  }

  if (e.key === 'ArrowUp') {
    e.preventDefault()
    e.stopPropagation()
    completion.selectPrev()

    return
  }

  if (e.key === 'Tab') {
    e.preventDefault()
    e.stopPropagation()

    // Tab NEVER auto-applies. The popover opens with no row tinted
    // (selectedIndex = -1 sentinel); first Tab walks the sentinel
    // onto row 0 so the captain sees the highlight; subsequent
    // Tabs walk the cursor through the list. Enter is the only
    // commit verb — auto-applying on Tab was buggy because a stray
    // Tab (e.g. while typing alongside the popover) inserted a
    // path the captain didn't pick.
    completion.selectNext()

    return
  }

  if (e.key === 'Enter') {
    // Only commit when a row is explicitly selected — otherwise let
    // Enter fall through to the chat-submit keybind. This is the
    // captain's fix: with auto-select-first, Enter on an open popover
    // was ambiguous (submit vs commit-completion). Now Enter always
    // means submit unless the captain has Tab/Arrow'd onto a row.
    if (completion.state.value.selectedIndex >= 0) {
      e.preventDefault()
      e.stopPropagation()
      applyCompletion()
    } else {
      completion.close()
    }

    return
  }

  if (e.key === 'Escape') {
    e.preventDefault()
    e.stopPropagation()
    completion.close()

    return
  }
}

// Estimated popover height (240px list + ~30px footer + 0px gap).
// Used to flip above the caret when the default below-placement would
// clip the popover off the viewport bottom. Slightly over-sized so a
// pixel-tight viewport still flips when the popover would just barely
// fit — the visual cost of an extra flip is zero.
const POPOVER_HEIGHT_ESTIMATE = 280
// VS Code-style: popover sits flush against the line below the caret.
// Any non-zero gap reads as a visible "floating" panel rather than an
// editor affordance.
const POPOVER_GAP = 0

/**
 * Recompute the popover's anchor coords against the current caret.
 * Pure layout — does NOT fire a completion query. Use on cursor-move
 * events (click, keyup over Home/End, etc.) where we want the open
 * popover to follow the caret without re-querying the daemon.
 */
function repositionPopover(): void {
  const el = textareaRef.value

  if (!el) {
    return
  }
  const cursor = el.selectionStart ?? el.value.length
  const coord = getCaretCoordinates(el, cursor)
  const below = coord.top + coord.height + POPOVER_GAP
  const wouldClipBelow = below + POPOVER_HEIGHT_ESTIMATE > window.innerHeight

  if (wouldClipBelow) {
    // Anchor from the bottom: popover's own bottom edge sits
    // POPOVER_GAP above the caret line top, regardless of how many
    // rows it ends up rendering.
    completionTop.value = null
    completionBottom.value = window.innerHeight - coord.top + POPOVER_GAP
  } else {
    completionTop.value = below
    completionBottom.value = null
  }
  completionLeft.value = coord.left
}

function fireCompletionQuery(opts?: { manual?: boolean }): void {
  const el = textareaRef.value

  if (!el) {
    return
  }
  repositionPopover()
  const cursor = el.selectionStart ?? el.value.length
  // Without an explicit cwd the daemon falls through to its own
  // `current_dir()` — wherever `hyprpilot daemon` was launched —
  // and ripgrep walks an unrelated tree.
  const activeId = useActiveInstance().id.value
  const sessionInfo = useSessionInfo(activeId).info
  const cwdFallback = useDaemonCwd().daemonCwd

  completion.query(el.value, cursor, {
    manual: opts?.manual ?? false,
    cwd: sessionInfo.value.cwd ?? cwdFallback.value,
    instanceId: activeId
  })
}

function onTextareaInput(): void {
  fireCompletionQuery()
}

/**
 * Cursor-move events (click / Home / End / PageUp / PageDown / mobile
 * tap-to-position). Arrow keys are intercepted by the popover's keymap
 * when it's open, so they never reach this path.
 *
 * Re-evaluate the trigger context, not just reposition. The captain's
 * caret may have jumped OUT of the current completion token (e.g.,
 * tapping mid-paragraph on mobile while the `@./foo` popover was up).
 * `fireCompletionQuery` re-runs the trigger matcher and closes the
 * popover when no trigger covers the new caret position; when it
 * still does, it repositions + refreshes the result set. The
 * `!open` short-circuit keeps a closed popover dormant (no daemon
 * round-trip per cursor move).
 */
function onTextareaCursorMove(): void {
  if (!completion.state.value.open) {
    return
  }
  fireCompletionQuery()
}

/**
 * `selectionchange` is the only DOM signal mobile WebViews fire when
 * the captain taps a different position inside an already-focused
 * textarea — neither `click` nor `keyup` is guaranteed. Without this
 * the popover would sit pinned at its old anchor (and at its old
 * stale token-context) while the caret moved away from it.
 *
 * Filtered to our textarea via `document.activeElement` — the event
 * fires for every selection change in the document including
 * `<select>` widgets, contenteditable nodes etc.
 */
function onDocumentSelectionChange(): void {
  if (document.activeElement !== textareaRef.value) {
    return
  }
  onTextareaCursorMove()
}

function applyCompletion(): void {
  const item = completion.commit()

  if (!item) {
    return
  }
  const el = textareaRef.value

  if (!el) {
    return
  }
  const before = el.value.slice(0, item.replacement.range.start)
  const after = el.value.slice(item.replacement.range.end)

  // `@./foo.ts` shape: path completion preceded by `@` (no space) →
  // attach the file content as a wire-side `Attachment` instead of
  // pasting the path text. Captain types `@./` to browse the cwd as
  // a file picker; commit a leaf and the body lands as part of the
  // next turn. Directory commits fall through to the regular insert
  // path (paste it so the captain can pick deeper).
  const trigger = before.endsWith('@') ? '@' : ''
  const isFileLeaf = item.kind === CompletionKind.Path && item.detail === 'file'

  if (trigger && isFileLeaf) {
    const path = item.replacement.text

    // Drop the `@` along with the path token so the textarea reads
    // clean — the attachment pill replaces it visually.
    const cleaned = before.slice(0, before.length - 1) + after

    text.value = cleaned
    void nextTick(() => {
      if (textareaRef.value) {
        const pos = cleaned.length === 0 ? 0 : before.length - 1

        textareaRef.value.setSelectionRange(pos, pos)
        textareaRef.value.focus()
      }
    })
    void invoke(TauriCommand.ReadFileForAttachment, { path })
      .then((res) => {
        const filename = (path.split('/').pop() || path).slice(0, 48)
        const truncatedNote = res.truncated ? ' (truncated)' : ''
        const slug = `file:${path}${truncatedNote}`

        // Push to the wire-side attachment queue. `attachmentPills`
        // (computed off `attachments.pending`) projects this onto a
        // visible pill in `pillsToRender` — calling `composer.addPill`
        // on top would render the file twice (one for each list).
        // `useAttachments` is the single-source-of-truth for both
        // display and wire transmission.
        attachments.add({
          slug,
          path: res.path,
          body: res.body,
          title: filename
        })
      })
      .catch((err) => {
        log.warn('composer: attachment read failed', { path, err: String(err) })
        pushToast(ToastTone.Err, `attach ${path}: ${String(err)}`)
      })

    return
  }

  const inserted = item.replacement.text

  text.value = before + inserted + after
  void nextTick(() => {
    if (textareaRef.value) {
      const pos = before.length + inserted.length

      textareaRef.value.setSelectionRange(pos, pos)
      textareaRef.value.focus()
    }
  })
}

onMounted(() => {
  resize()
  composer.registerTextarea(textareaRef.value)
  document.addEventListener('selectionchange', onDocumentSelectionChange)
})

onUnmounted(() => {
  composer.registerTextarea(undefined)
  document.removeEventListener('selectionchange', onDocumentSelectionChange)
})

watch(text, () => nextTick(resize))

defineExpose({
  clear(): void {
    composer.clear()
    nextTick(resize)
  },
  addPill(pill: ComposerPill): void {
    composer.addPill(pill)
  },
  setDraft(next: { text: string; pills: ComposerPill[] }): void {
    composer.setDraft(next)
    nextTick(resize)
  }
})

function trySubmit(): void {
  if (props.sending || props.disabled) {
    return
  }

  // Require non-empty BUFFER on every submit. Whitespace counts as
  // text — `resolvedSubmit` trims for the wire (so a "  hi  " buffer
  // ships as "hi"), but here we gate off the raw textarea value so
  // a captain who deliberately typed spaces (e.g. as a leading-newline
  // workaround on a coarse keyboard) can still send. The wire-side
  // payload is whatever `resolvedSubmit` produced — we only care here
  // that SOMETHING was typed. Attachments + pills alone are not enough:
  // an attachments-only turn lands on the daemon as
  // `ContentBlock::Resource[]` with no user text, agents dispatch it
  // as if the captain said nothing, and the next prompt's reply
  // attaches to the wrong turn id.
  if (composer.text.value.length === 0) {
    return
  }
  const { text, attachments } = composer.resolvedSubmit()

  // Drop any open completion before the buffer clears — without
  // this the popover stays mounted with results computed against
  // the just-submitted text.
  completion.close()
  emit('submit', { text, attachments })
}

function onEnter(e: KeyboardEvent): boolean {
  if (e.isComposing) {
    return false
  }

  // Mobile branch: a soft keyboard has no Shift, so binding submit to
  // bare Enter strands the captain in single-line mode. Fall through
  // and let the textarea insert a newline; the visible send button is
  // the submit path. Desktop keeps the Enter-to-send muscle memory.
  if (touch.isCoarsePointer.value) {
    return false
  }
  log.debug('composer keybind', { key: 'Enter' })
  trySubmit()

  return true
}

function onTab(): boolean {
  log.debug('composer keybind', { key: 'Tab', target: 'completion' })

  // When the popover is open, `onTextareaKeydown` already handled
  // Tab and prevented default; the keymap chain shouldn't run. When
  // closed, Tab here means "force-open completion" (manual ripgrep
  // trigger). Either way we swallow the event from the keymap chain.
  if (completion.state.value.open) {
    return true
  }
  fireCompletionQuery({ manual: true })

  return true
}

async function readClipboardImagePill(): Promise<ComposerPill | undefined> {
  try {
    const image = await readImage()
    const rgba = await image.rgba()
    const { width, height } = await image.size()
    const blob = await rgbaToPngBlob(rgba, width, height)

    if (!blob) {
      return undefined
    }
    const dataUrl = await blobToDataUrl(blob)

    return {
      kind: ComposerPillKind.Attachment,
      id: crypto.randomUUID(),
      label: `image/png · ${formatSize(blob.size)}`,
      data: dataUrl.slice(dataUrl.indexOf(',') + 1),
      mimeType: 'image/png'
    }
  } catch(err) {
    log.debug('clipboard readImage failed', { err: String(err) })

    return undefined
  }
}

function onPasteImage(e: KeyboardEvent): boolean {
  log.debug('composer keybind', { key: 'ctrl+p', target: 'paste-image' })
  e.preventDefault()

  void (async() => {
    attachmentLoading.value += 1

    try {
      const pill = await readClipboardImagePill()

      if (pill) {
        composer.addPill(pill)
      }
    } finally {
      attachmentLoading.value = Math.max(0, attachmentLoading.value - 1)
    }
  })()

  return true
}

/**
 * Trigger the hidden `<input type="file">` so the OS file picker
 * opens. The change handler reads each picked image into a
 * composer pill, mirroring the drag-drop path. Non-image files
 * are dropped per the skill-only-via-palette convention.
 */
function onAttachClick(): void {
  fileInputRef.value?.click()
}

/**
 * Mime types the daemon's `attachment_to_block` encoder routes
 * through `TextResourceContents` (reads `att.body`, ignores
 * `att.data`). For these, the UI reads the file via `readAsText`
 * and puts the plain content into the pill's `data` field — the
 * projector (`Overlay.vue::pillsToAttachments`) then ships it as
 * the wire `Attachment.body` instead of `Attachment.data`.
 *
 * Everything else (image / audio / pdf / archive / arbitrary
 * binary) reads as a data URL → base64 → wire `Attachment.data`,
 * and the daemon dispatches to `ImageContent` / `AudioContent` /
 * `BlobResourceContents` based on the mime category.
 */
function isTextMime(mime: string): boolean {
  if (mime.startsWith('text/')) {
    return true
  }

  // Structured text formats the daemon's `mime_category` rule treats
  // as Text. Keep this list in lockstep with
  // `acp/instance.rs::mime_category`.
  return mime === 'application/json' || mime === 'application/xml' || mime === 'application/x-yaml' || mime === 'application/toml'
}

async function onFileInputChange(e: Event): Promise<void> {
  const input = e.target as HTMLInputElement
  const files = input.files

  if (!files || files.length === 0) {
    return
  }

  for (const file of Array.from(files)) {
    attachmentLoading.value += 1

    try {
      // Text-shaped mimes ship as `body` (plain text); everything
      // else ships as `data` (base64 binary). The pill carries one
      // single `data` string field — the projector branches on
      // `mimeType` to decide which wire field to populate.
      const mime = file.type || 'application/octet-stream'
      const payload = isTextMime(mime) ? await file.text() : ((await blobToDataUrl(file)).split(',')[1] ?? '')

      composer.addPill({
        kind: ComposerPillKind.Attachment,
        id: crypto.randomUUID(),
        label: `${file.name || mime} · ${formatSize(file.size)}`,
        data: payload,
        mimeType: mime,
        fileName: file.name || undefined
      })
    } catch(err) {
      log.warn('composer attach: file read failed', { name: file.name, err: String(err) })
    } finally {
      attachmentLoading.value = Math.max(0, attachmentLoading.value - 1)
    }
  }
  // Reset the input so re-picking the same file fires `change` again.
  input.value = ''
}

function onHistoryPrev(): boolean {
  log.debug('composer keybind', { key: 'ctrl+arrowup', target: 'history-prev' })

  return false
}

function onHistoryNext(): boolean {
  log.debug('composer keybind', { key: 'ctrl+arrowdown', target: 'history-next' })

  return false
}

function onRemovePill(id: string): void {
  if (id.startsWith('attachment:')) {
    attachments.remove(id.slice('attachment:'.length))
  } else {
    composer.removePill(id)
  }
  emit('removePill', id)
}

async function onDrop(e: DragEvent): Promise<void> {
  e.preventDefault()
  const files = e.dataTransfer?.files

  if (!files || files.length === 0) {
    return
  }

  for (const file of Array.from(files)) {
    if (!file.type.startsWith('image/')) {
      // Skill / reference attachments are palette-driven; the
      // composer doesn't accept ad-hoc resource drops.
      continue
    }
    attachmentLoading.value += 1

    try {
      const dataUrl = await blobToDataUrl(file)

      composer.addPill({
        kind: ComposerPillKind.Attachment,
        id: crypto.randomUUID(),
        label: `${file.name || file.type} · ${formatSize(file.size)}`,
        data: dataUrl.slice(dataUrl.indexOf(',') + 1),
        mimeType: file.type,
        fileName: file.name || undefined
      })
    } finally {
      attachmentLoading.value = Math.max(0, attachmentLoading.value - 1)
    }
  }
}

function onDragOver(e: DragEvent): void {
  if (e.dataTransfer) {
    e.dataTransfer.dropEffect = 'copy'
    e.preventDefault()
  }
}
</script>

<template>
  <form class="composer" data-testid="composer" @submit.prevent="() => void trySubmit()" @drop="onDrop" @dragover="onDragOver">
    <div v-if="pillsToRender.length > 0 || attachmentLoading > 0" class="composer-pills">
      <ChatComposerPill v-for="p in pillsToRender" :key="p.id" :pill="p" @remove="onRemovePill" />
      <span v-if="attachmentLoading > 0" class="composer-pill-loading" data-testid="composer-attaching">
        <FaIcon :icon="faCircleNotch" class="composer-pill-loading-icon animate-spin" aria-hidden="true" />
        attaching{{ attachmentLoading > 1 ? ` ${attachmentLoading} files` : '…' }}
      </span>
    </div>

    <!-- Hidden file picker — `accept="image/*"` mirrors the
         drag-drop guard. Multiple to mirror the loop in onDrop. -->
    <input ref="fileInputRef" type="file" multiple hidden data-testid="composer-file-input" @change="(e) => void onFileInputChange(e)" />

    <div class="composer-row">
      <textarea
        ref="textareaRef"
        v-model="text"
        class="composer-textarea"
        :placeholder="placeholder"
        :disabled="disabled"
        autocapitalize="sentences"
        autocorrect="on"
        spellcheck="true"
        :enterkeyhint="touch.isCoarsePointer.value ? 'enter' : 'send'"
        data-testid="composer-textarea"
        @keydown.capture="onTextareaKeydown"
        @input="onTextareaInput"
        @click="onTextareaCursorMove"
        @keyup="onTextareaCursorMove"
        @blur="completion.close()"
      />
      <CompletionPopover :top="completionTop" :bottom="completionBottom" :left="completionLeft" @commit="applyCompletion" />
      <div class="composer-actions">
        <button
          type="submit"
          class="composer-submit"
          :aria-label="sending ? 'sending' : 'send'"
          :data-empty="text.length === 0"
          :disabled="sending || disabled || text.length === 0"
          data-testid="composer-submit"
        >
          <FaIcon :icon="faArrowTurnDown" class="composer-action-icon" aria-hidden="true" />
        </button>
        <button
          v-if="canCancel"
          type="button"
          class="composer-cancel"
          aria-label="cancel current turn"
          title="cancel (Ctrl+C)"
          data-testid="composer-cancel"
          @click="emit('cancel')"
        >
          <FaIcon :icon="faStop" class="composer-action-icon" aria-hidden="true" />
        </button>
        <button type="button" class="composer-attach" aria-label="attach image" :disabled="disabled" data-testid="composer-attach" @click="onAttachClick">
          <FaIcon :icon="faPaperclip" class="composer-action-icon" aria-hidden="true" />
        </button>
      </div>
    </div>
  </form>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* composer: surface bg + line top border, padding 8px 14px, vertical
 * stack of attachment pills (when present) + input row. */
.composer {
  @apply flex flex-col;
  background-color: var(--theme-surface);
  border-top: 1px solid var(--theme-border);
  padding: 0.5rem 0.875rem;
  gap: 0.3125rem;
}

.composer-pills {
  @apply flex flex-wrap items-center gap-1;
}

/* In-flight attachment placeholder pill — shape mirrors a real
 * attachment pill so the row doesn't reflow when the FileReader
 * settles and the real pill takes its place. Spinner + lowercase
 * "attaching…" copy reads as transient. */
.composer-pill-loading {
  @apply inline-flex items-center gap-[0.375rem] text-[0.62rem] uppercase;
  background-color: var(--theme-surface-bg);
  border: 1px dashed var(--theme-border-soft);
  color: var(--theme-fg-dim);
  padding: 0.1875rem 0.5rem;
  border-radius: 0.1875rem;
  letter-spacing: 0.03125rem;
  font-family: var(--theme-font-mono);
}

.composer-pill-loading-icon {
  width: 0.5625rem;
  height: 0.5625rem;
  color: var(--theme-accent);
}

.composer-row {
  @apply flex items-stretch;
  min-width: 0;
  gap: 0.375rem;
}

/* Textarea: bg-bg, line2 border, padding 8px 10px. min-height 96px
 * gives the autocomplete popover vertical room before it has to flip
 * above. */
.composer-textarea {
  @apply w-full min-w-0 flex-1 resize-none overflow-y-auto border text-[0.75rem] leading-snug;
  font-family: var(--theme-font-mono);
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg);
  border-color: var(--theme-border-soft);
  border-radius: 0.25rem;
  padding: 0.5rem 0.625rem;
  min-height: 6rem;
  /* Cap at 50% of the viewport height so a captain pasting a long
   * spec / fenced code block can keep most of it in view while
   * editing instead of scrolling inside a stubby 25vh box. */
  max-height: 50dvh;
}

.composer-textarea::placeholder {
  color: var(--theme-fg-dim);
}

.composer-textarea:focus {
  outline: none;
  border-color: var(--theme-accent);
}

.composer-textarea:disabled {
  opacity: 0.5;
}

/* Vertical button cluster: 44px wide, send + attach stacked. */
.composer-actions {
  @apply flex shrink-0 flex-col;
  width: 2.75rem;
  gap: 0.25rem;
}

/* Send: solid accent when there's content, ghost otherwise. */
.composer-submit {
  @apply flex flex-1 items-center justify-center font-bold text-[0.85rem];
  font-family: var(--theme-font-mono);
  background-color: var(--theme-accent);
  color: var(--theme-fg-on-tone);
  border: 1px solid var(--theme-accent);
  border-radius: 0.25rem;
  cursor: pointer;
}

.composer-submit[data-empty='true'] {
  background-color: transparent;
  color: var(--theme-accent);
}

.composer-submit:hover:not(:disabled) {
  filter: brightness(1.1);
}

.composer-submit:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* Shared icon size for the stacked composer buttons (send / cancel /
 * attach). Each glyph sits in a 22px square so the buttons stack
 * cleanly without baseline jitter. */
.composer-action-icon {
  width: 0.75rem;
  height: 0.75rem;
}

/* Cancel — red ghost stop button. Renders only while a turn is in
 * flight (parent passes `:can-cancel`). Mirrors the attach button's
 * shape so the button stack reads as a uniform action column. */
.composer-cancel {
  @apply flex items-center justify-center;
  height: 1.375rem;
  background-color: transparent;
  color: var(--theme-status-err);
  border: 1px solid var(--theme-status-err);
  border-radius: 0.25rem;
  cursor: pointer;
  transition: background-color 0.12s ease-out;
}

.composer-cancel:hover {
  background-color: var(--theme-status-err);
  color: var(--theme-fg-on-tone);
}

/* Attach: always ghost. */
.composer-attach {
  @apply flex items-center justify-center text-[0.7rem];
  height: 1.375rem;
  background-color: transparent;
  color: var(--theme-fg-dim);
  border: 1px solid var(--theme-border-soft);
  border-radius: 0.25rem;
  font-family: var(--theme-font-mono);
  cursor: pointer;
}

.composer-attach:hover:not(:disabled) {
  color: var(--theme-fg);
  border-color: var(--theme-fg-dim);
}

/* Touch hit-area floor on phones — bump the cancel + attach buttons
 * (and let the send button grow to fill since it's `flex-1`). */
@media (pointer: coarse) {
  .composer-cancel,
  .composer-attach {
    height: auto;
    min-height: 2.5rem;
  }

  .composer-actions {
    width: 3.25rem;
    gap: 0.375rem;
  }
}

.composer-attach:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>
