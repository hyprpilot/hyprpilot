/**
 * Composer state owner: text buffer + image-attachment pill list. Each
 * `useComposer()` call returns its own factory state — that's the
 * right shape for a textarea-local concern. Cross-component sharing
 * (the skills palette → composer attachment chain) goes through
 * `useAttachments` (a module-scope singleton); the composer reads
 * `useAttachments().pending` for pill rendering, but it doesn't own
 * that store.
 *
 * Inline `#{kind/slug}` token expansion was deleted in K-268's pivot
 * to palette-only skill delivery; resources now travel as first-class
 * `Attachment` entries on the user turn (`ContentBlock::Resource` on
 * the wire), not as text-inlined sections.
 */

import { ref, type Ref } from 'vue'

import { type ComposerPill } from '@components/types'

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
}

export function useComposer(): ComposerState {
  const text = ref('')
  const pills = ref<ComposerPill[]>([])

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

  return {
    text,
    pills,
    addPill,
    removePill,
    clear,
    resolvedSubmit
  }
}
