<script setup lang="ts">
import { faFile, faFileCode, faFileImage, faFileLines, faFolder, faUpRightFromSquare } from '@fortawesome/free-solid-svg-icons'
import { computed } from 'vue'

import type { Attachment } from '@ipc'

/**
 * Pill-row of attachments the captain submitted with a user turn.
 * Each pill: icon + last-segment name + mimetype tag. Click emits
 * `open` so the host can dispatch (typically to
 * `tauri-plugin-shell::open` for the file's default app).
 *
 * Rendering is intentionally compact — bodies are delivered to the
 * agent over the wire as embedded resources; the UI just shows a
 * receipt that "this context went along with the turn", not the
 * full body. Image attachments still inline (one tile per image)
 * because the captain composed them visually and expects to see
 * them as visuals, not a "image · png" pill.
 */
const props = defineProps<{
  attachments: Attachment[]
}>()

const emit = defineEmits<{
  open: [att: Attachment]
}>()

interface AttachmentView {
  att: Attachment
  name: string
  mime: string
  icon: typeof faFile
}

function iconFor(mime: string | undefined): typeof faFile {
  if (!mime) {
    return faFile
  }
  if (mime.startsWith('image/')) {
    return faFileImage
  }
  if (mime.startsWith('text/markdown') || mime.startsWith('text/x-markdown')) {
    return faFileLines
  }
  if (mime.startsWith('text/') || mime === 'application/json' || mime === 'application/xml') {
    return faFileCode
  }
  if (mime === 'inode/directory') {
    return faFolder
  }
  return faFile
}

function nameFor(att: Attachment): string {
  if (att.title && att.title.length > 0) {
    return att.title
  }
  if (att.path) {
    const seg = att.path.split('/').filter(Boolean).pop()
    if (seg) {
      return seg
    }
  }
  return att.slug || '<attachment>'
}

function mimeFor(att: Attachment): string {
  if (att.mime && att.mime.length > 0) {
    return att.mime
  }
  if (att.body !== undefined) {
    return 'text/plain'
  }
  return 'unknown'
}

function isImage(att: Attachment): boolean {
  return typeof att.mime === 'string' && att.mime.startsWith('image/') && typeof att.data === 'string'
}

function imageSrc(att: Attachment): string {
  return `data:${att.mime ?? 'image/png'};base64,${att.data ?? ''}`
}

const views = computed<AttachmentView[]>(() =>
  props.attachments.map((att) => ({
    att,
    name: nameFor(att),
    mime: mimeFor(att),
    icon: iconFor(att.mime)
  }))
)

const images = computed(() => props.attachments.filter(isImage))
</script>

<template>
  <div v-if="views.length > 0" class="attachments">
    <div class="attachments-pills">
      <button
        v-for="v in views"
        :key="v.att.slug || v.att.path || v.name"
        type="button"
        class="attachments-pill"
        :title="v.att.path ?? v.name"
        @click="emit('open', v.att)"
      >
        <FaIcon :icon="v.icon" class="attachments-pill-icon" aria-hidden="true" />
        <span class="attachments-pill-name">{{ v.name }}</span>
        <span class="attachments-pill-mime">{{ v.mime }}</span>
        <FaIcon :icon="faUpRightFromSquare" class="attachments-pill-open" aria-hidden="true" />
      </button>
    </div>
    <div v-if="images.length > 0" class="attachments-images">
      <img
        v-for="(img, i) in images"
        :key="i"
        :src="imageSrc(img)"
        :alt="nameFor(img)"
        class="attachments-image"
      />
    </div>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.attachments {
  @apply flex flex-col;
  margin-top: 4px;
  gap: 6px;
}

.attachments-pills {
  @apply flex flex-wrap;
  gap: 4px;
}

.attachments-pill {
  @apply inline-flex items-center gap-2 border bg-transparent;
  border-color: var(--theme-border-soft);
  border-radius: 999px;
  padding: 2px 8px;
  font-family: var(--theme-font-mono);
  font-size: 0.62rem;
  color: var(--theme-fg-ink-2);
  cursor: pointer;
  max-width: 100%;
  min-width: 0;
}

.attachments-pill:hover {
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
}

.attachments-pill-icon {
  width: 9px;
  height: 9px;
  color: var(--theme-fg-dim);
  flex-shrink: 0;
}

.attachments-pill-name {
  @apply truncate;
  color: var(--theme-fg);
  font-weight: 600;
  max-width: 220px;
}

.attachments-pill-mime {
  color: var(--theme-fg-faint);
  letter-spacing: 0.3px;
  font-size: 0.58rem;
}

.attachments-pill-open {
  width: 8px;
  height: 8px;
  color: var(--theme-fg-faint);
  flex-shrink: 0;
}

.attachments-pill:hover .attachments-pill-open {
  color: var(--theme-accent);
}

.attachments-images {
  @apply flex flex-wrap;
  gap: 4px;
}

.attachments-image {
  @apply rounded-sm;
  max-width: 100%;
  max-height: 240px;
  object-fit: contain;
  border: 1px solid var(--theme-border-soft);
  background-color: var(--theme-surface);
}
</style>
