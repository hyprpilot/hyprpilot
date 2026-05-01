<script setup lang="ts">
/**
 * Composer row: pills (image attachments + skill attachments) +
 * autosizing textarea + send button. Owns compose text + image-pill
 * state via `useComposer`; reads skill attachments off the
 * `useAttachments` module-scope singleton (the K-268 skills palette
 * pushes there). The parent's `@submit` receives `{ text, attachments }`
 * — image pills go in the `attachments` slot, skill attachments
 * travel separately via `useAttachments().pending`.
 *
 * Ctrl+P (`composer.paste_image` binding) reads a clipboard image via
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
import { ComposerPillKind, type ComposerPill } from '@components'
import {
  type KeymapEntry,
  useAttachments,
  useCompletion,
  useComposer,
  useKeymap,
  useKeymaps
} from '@composables'
import { Modifier } from '@ipc'
import { getCaretCoordinates, log } from '@lib'


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

// Skill attachments (palette-driven, K-268) render as resource pills
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
    { binding: keymaps.value.composer.paste_image, handler: onPasteImage },
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
    { binding: keymaps.value.composer.history_up, handler: onHistoryPrev, allowRepeat: true },
    { binding: keymaps.value.composer.history_down, handler: onHistoryNext, allowRepeat: true }
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
  if (e.key === 'Enter' || e.key === 'Tab') {
    e.preventDefault()
    e.stopPropagation()
    applyCompletion()
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
  completion.query(el.value, cursor, { manual: opts?.manual ?? false })
}

function onTextareaInput(): void {
  fireCompletionQuery()
}

/**
 * Cursor-move events (click / Home / End / PageUp / PageDown). Arrow
 * keys are intercepted by the popover's keymap when it's open, so they
 * never reach this path. We only need to reposition when the popover
 * is already open — closed popover doesn't render anywhere to move.
 */
function onTextareaCursorMove(): void {
  if (completion.state.value.open) {
    repositionPopover()
  }
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
})

onUnmounted(() => {
  composer.registerTextarea(undefined)
})

watch(text, () => nextTick(resize))

defineExpose({
  clear(): void {
    composer.clear()
    nextTick(resize)
  },
  addPill(pill: ComposerPill): void {
    composer.addPill(pill)
  }
})

function trySubmit(): void {
  if (props.sending || props.disabled) {
    return
  }
  const { text, attachments } = composer.resolvedSubmit()
  if (!text && attachments.length === 0) {
    return
  }
  emit('submit', { text, attachments })
}

function onEnter(e: KeyboardEvent): boolean {
  if (e.isComposing) {
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
  } catch (err) {
    log.debug('clipboard readImage failed', { err: String(err) })

    return undefined
  }
}

function onPasteImage(e: KeyboardEvent): boolean {
  log.debug('composer keybind', { key: 'ctrl+p', target: 'paste-image' })
  e.preventDefault()

  void (async () => {
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

async function onFileInputChange(e: Event): Promise<void> {
  const input = e.target as HTMLInputElement
  const files = input.files
  if (!files || files.length === 0) {
    return
  }
  for (const file of Array.from(files)) {
    if (!file.type.startsWith('image/')) {
      log.debug('composer attach: skipping non-image file', { name: file.name, type: file.type })
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
    } catch (err) {
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

/** RGBA pixel buffer → PNG blob via offscreen canvas. */
async function rgbaToPngBlob(rgba: Uint8Array, width: number, height: number): Promise<Blob | undefined> {
  if (width === 0 || height === 0) {
    return undefined
  }
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')
  if (!ctx) {
    return undefined
  }
  // Copy into a fresh `Uint8ClampedArray` (own ArrayBuffer) so the
  // TS lib's `ImageDataArray` parameter type accepts it — recent
  // lib.dom.d.ts narrows to `Uint8ClampedArray<ArrayBuffer>`, while
  // the view-of-rgba.buffer reads as `ArrayBufferLike` (which the
  // SharedArrayBuffer branch rejects).
  const data = new Uint8ClampedArray(rgba.byteLength)
  data.set(new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength))
  ctx.putImageData(new ImageData(data, width, height), 0, 0)

  return new Promise((resolve) => {
    canvas.toBlob((blob) => resolve(blob ?? undefined), 'image/png')
  })
}

/** FileReader-based base64 dataURL — async, off the main thread. */
function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader()
    r.onload = () => resolve(r.result as string)
    r.onerror = () => reject(r.error)
    r.readAsDataURL(blob)
  })
}

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)}MB`
  }
  if (bytes >= 1024) {
    return `${(bytes / 1024).toFixed(1)}KB`
  }

  return `${Math.round(bytes)}B`
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
      // Skill / reference attachments are palette-driven (K-268); the
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
    <input
      ref="fileInputRef"
      type="file"
      accept="image/*"
      multiple
      hidden
      data-testid="composer-file-input"
      @change="(e) => void onFileInputChange(e)"
    />

    <div class="composer-row">
      <textarea
        ref="textareaRef"
        v-model="text"
        class="composer-textarea"
        rows="3"
        :placeholder="placeholder"
        :disabled="disabled"
        data-testid="composer-textarea"
        @keydown.capture="onTextareaKeydown"
        @input="onTextareaInput"
        @click="onTextareaCursorMove"
        @keyup="onTextareaCursorMove"
        @blur="completion.close()"
      />
      <CompletionPopover
        :top="completionTop"
        :bottom="completionBottom"
        :left="completionLeft"
        @commit="applyCompletion"
      />
      <div class="composer-actions">
        <button
          type="submit"
          class="composer-submit"
          :aria-label="sending ? 'sending' : 'send'"
          :data-empty="text.trim().length === 0 && composerPills.length === 0 && attachments.pending.value.length === 0"
          :disabled="sending || disabled || (text.trim().length === 0 && composerPills.length === 0 && attachments.pending.value.length === 0)"
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
        <button
          type="button"
          class="composer-attach"
          aria-label="attach image"
          :disabled="disabled"
          data-testid="composer-attach"
          @click="onAttachClick"
        >
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
  padding: 8px 14px;
  gap: 5px;
}

.composer-pills {
  @apply flex flex-wrap items-center gap-1;
}

/* In-flight attachment placeholder pill — shape mirrors a real
 * attachment pill so the row doesn't reflow when the FileReader
 * settles and the real pill takes its place. Spinner + lowercase
 * "attaching…" copy reads as transient. */
.composer-pill-loading {
  @apply inline-flex items-center gap-[6px] text-[0.62rem] uppercase;
  background-color: var(--theme-surface-bg);
  border: 1px dashed var(--theme-border-soft);
  color: var(--theme-fg-dim);
  padding: 3px 8px;
  border-radius: 3px;
  letter-spacing: 0.5px;
  font-family: var(--theme-font-mono);
}

.composer-pill-loading-icon {
  width: 9px;
  height: 9px;
  color: var(--theme-accent);
}

.composer-row {
  @apply flex items-stretch;
  min-width: 0;
  gap: 6px;
}

/* wireframe textarea-equivalent: bg-bg, line2 border, padding 8px 10px.
 * min-height bumped past wireframe (68→96) to give the autocomplete
 * popover a bit more vertical room before it has to flip above. */
.composer-textarea {
  @apply w-full min-w-0 flex-1 resize-none overflow-y-auto border text-[0.75rem] leading-snug;
  font-family: var(--theme-font-mono);
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg);
  border-color: var(--theme-border-soft);
  border-radius: 4px;
  padding: 8px 10px;
  min-height: 96px;
  max-height: 25vh;
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

/* wireframe vertical button cluster: 44px wide, send + attach stacked. */
.composer-actions {
  @apply flex flex-col;
  width: 44px;
  gap: 4px;
}

/* wireframe send: solid yellow accent when content, ghost otherwise. */
.composer-submit {
  @apply flex flex-1 items-center justify-center font-bold text-[0.85rem];
  font-family: var(--theme-font-mono);
  background-color: var(--theme-accent);
  color: var(--theme-fg-on-tone);
  border: 1px solid var(--theme-accent);
  border-radius: 4px;
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
  width: 12px;
  height: 12px;
}

/* Cancel — red ghost stop button. Renders only while a turn is in
 * flight (parent passes `:can-cancel`). Mirrors the attach button's
 * shape so the button stack reads as a uniform action column. */
.composer-cancel {
  @apply flex items-center justify-center;
  height: 22px;
  background-color: transparent;
  color: var(--theme-status-err);
  border: 1px solid var(--theme-status-err);
  border-radius: 4px;
  cursor: pointer;
  transition: background-color 0.12s ease-out;
}

.composer-cancel:hover {
  background-color: var(--theme-status-err);
  color: var(--theme-fg-on-tone);
}

/* wireframe attach: always ghost. */
.composer-attach {
  @apply flex items-center justify-center text-[0.7rem];
  height: 22px;
  background-color: transparent;
  color: var(--theme-fg-dim);
  border: 1px solid var(--theme-border-soft);
  border-radius: 4px;
  font-family: var(--theme-font-mono);
  cursor: pointer;
}

.composer-attach:hover:not(:disabled) {
  color: var(--theme-fg);
  border-color: var(--theme-fg-dim);
}

.composer-attach:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>
