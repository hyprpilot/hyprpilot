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
import { readImage } from '@tauri-apps/plugin-clipboard-manager'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'

import ChatComposerPill from './ChatComposerPill.vue'
import { ComposerPillKind, type ComposerPill } from '../types'
import { type KeymapEntry, useAttachments, useComposer, useKeymap, useKeymaps } from '@composables'
import { log } from '@lib'


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
  }>(),
  {
    placeholder: 'message pilot',
    disabled: false,
    sending: false,
    pills: undefined
  }
)

const emit = defineEmits<{
  submit: [payload: { text: string; attachments: ComposerPill[] }]
  removePill: [id: string]
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

const textareaRef = ref<HTMLTextAreaElement>()

function resize(): void {
  const el = textareaRef.value
  if (!el) {
    return
  }
  el.style.height = 'auto'
  el.style.height = `${el.scrollHeight}px`
}

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
    { binding: keymaps.value.composer.history_up, handler: onHistoryPrev, allowRepeat: true },
    { binding: keymaps.value.composer.history_down, handler: onHistoryNext, allowRepeat: true }
  ]
})

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

  return false
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
    const pill = await readClipboardImagePill()
    if (pill) {
      composer.addPill(pill)
    }
  })()

  return true
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
  const data = new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength)
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
    const dataUrl = await blobToDataUrl(file)
    composer.addPill({
      kind: ComposerPillKind.Attachment,
      id: crypto.randomUUID(),
      label: `${file.name || file.type} · ${formatSize(file.size)}`,
      data: dataUrl.slice(dataUrl.indexOf(',') + 1),
      mimeType: file.type
    })
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
    <div v-if="pillsToRender.length > 0" class="composer-pills">
      <ChatComposerPill v-for="p in pillsToRender" :key="p.id" :pill="p" @remove="onRemovePill" />
    </div>

    <div class="composer-row">
      <textarea
        ref="textareaRef"
        v-model="text"
        class="composer-textarea"
        rows="5"
        :placeholder="placeholder"
        :disabled="disabled"
        data-testid="composer-textarea"
      />
      <button
        type="submit"
        class="composer-submit"
        :aria-label="sending ? 'sending' : 'send'"
        :disabled="sending || disabled || (text.trim().length === 0 && composerPills.length === 0 && attachments.pending.value.length === 0)"
        data-testid="composer-submit"
      >
        <FaIcon :icon="['fas', 'reply']" class="composer-submit-icon" aria-hidden="true" />
      </button>
    </div>
  </form>
</template>

<style scoped>
@reference '../../assets/styles.css';

.composer {
  @apply flex flex-col gap-1 px-3 py-2;
  background-color: var(--theme-surface);
}

.composer-pills {
  @apply flex flex-wrap items-center gap-1;
}

.composer-row {
  @apply flex items-end gap-2;
  min-width: 0;
}

.composer-textarea {
  @apply w-full min-w-0 flex-1 resize-none overflow-y-auto border px-2 py-1 text-[0.85rem] leading-snug;
  font-family: var(--theme-font-mono);
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg);
  border-color: var(--theme-border);
  max-height: 25vh;
}

.composer-textarea:focus {
  outline: none;
  border-color: var(--theme-accent);
}

.composer-textarea:disabled {
  opacity: 0.5;
}

.composer-submit {
  @apply shrink-0 self-stretch border-0 px-[14px] py-2 font-bold text-[0.82rem];
  font-family: var(--theme-font-mono);
  color: var(--theme-surface-bg);
  background-color: var(--theme-accent-assistant);
  cursor: pointer;
}

.composer-submit-icon {
  width: 13px;
  height: 13px;
}

.composer-submit:hover:not(:disabled) {
  background-color: color-mix(in srgb, var(--theme-accent-assistant) 85%, var(--theme-surface-bg));
}

.composer-submit:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>
