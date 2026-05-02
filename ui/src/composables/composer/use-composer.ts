/**
 * Composer state owner — module-scope singleton. The compose row, the
 * skills palette (K-268), and the slash-commands palette (K-267) all
 * reach into the same buffer; sharing through Vue refs threaded down
 * the tree would be more ceremony than the single-window assumption
 * deserves. Per-test isolation goes through `__resetComposerForTests`,
 * mirroring `__resetPaletteStackForTests`.
 *
 * The textarea ref is registered by `ChatComposer.vue` on mount so
 * `insertAtCaret` can target the live caret position; before mount (or
 * after unmount) callers fall through to a buffer-end append. The
 * skill-attachment store still lives in `useAttachments` — composer
 * pills here are the image-attachment list.
 *
 * Inline `#{kind/slug}` token expansion was deleted in K-268's pivot
 * to palette-only skill delivery; resources travel as first-class
 * `Attachment` entries on the user turn (`ContentBlock::Resource` on
 * the wire), not as text-inlined sections.
 */

import { nextTick, ref, type Ref } from 'vue'

import { type ComposerPill } from '@components'

export interface ResolvedSubmit {
  text: string
  attachments: ComposerPill[]
}

export interface ComposerState {
  text: Ref<string>
  pills: Ref<ComposerPill[]>
  addPill: (pill: ComposerPill) => void
  removePill: (id: string) => void
  clear: () => void
  resolvedSubmit: () => ResolvedSubmit
  insertAtCaret: (snippet: string) => void
  registerTextarea: (el: HTMLTextAreaElement | undefined) => void
  focus: () => void
}

const text = ref('')
const pills = ref<ComposerPill[]>([])
let textareaEl: HTMLTextAreaElement | undefined

function addPill(pill: ComposerPill): void {
  if (pills.value.some((p) => p.id === pill.id)) {
    return
  }
  pills.value = [...pills.value, pill]
}

function removePill(id: string): void {
  pills.value = pills.value.filter((p) => p.id !== id)
}

function clear(): void {
  text.value = ''
  pills.value = []
}

function resolvedSubmit(): ResolvedSubmit {
  return { text: text.value.trim(), attachments: pills.value }
}

function registerTextarea(el: HTMLTextAreaElement | undefined): void {
  textareaEl = el
}

function focus(): void {
  textareaEl?.focus()
}

/**
 * Insert `snippet` at the textarea's selection range, replacing any
 * selected text. With no live textarea (palette opened before
 * `ChatComposer` mounted, or after teardown) the snippet appends to
 * the end of the buffer. Caret lands at the end of the inserted
 * snippet on the next tick once Vue patches the v-model.
 */
function insertAtCaret(snippet: string): void {
  const el = textareaEl

  if (!el) {
    text.value = `${text.value}${snippet}`

    return
  }
  const start = el.selectionStart ?? text.value.length
  const end = el.selectionEnd ?? text.value.length
  const before = text.value.slice(0, start)
  const after = text.value.slice(end)

  text.value = `${before}${snippet}${after}`
  const caret = before.length + snippet.length

  void nextTick(() => {
    el.focus()
    el.setSelectionRange(caret, caret)
  })
}

export function useComposer(): ComposerState {
  return {
    text,
    pills,
    addPill,
    removePill,
    clear,
    resolvedSubmit,
    insertAtCaret,
    registerTextarea,
    focus
  }
}

/** Test-only: clear text + pills + textarea registration. */
export function __resetComposerForTests(): void {
  text.value = ''
  pills.value = []
  textareaEl = undefined
}
