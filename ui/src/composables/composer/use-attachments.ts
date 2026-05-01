/**
 * Pending-attachment store. Module-scope singleton so the skills
 * palette (K-268) and the composer share one store without threading
 * refs through `Overlay.vue`. Submission flow:
 *
 *   1. Palette tick → `add({ slug, path, body, title })`.
 *   2. Composer renders pills off `pending`.
 *   3. `Overlay.vue::onSubmit` passes `pending.value` on `session_submit`.
 *   4. Submit-ack → `clear()`.
 *   5. Cancel mid-turn → keep populated (resubmit same set).
 *   6. Instance switch → `clear()`.
 *
 * Wire mapping: Rust adapter folds each entry onto an ACP
 * `ContentBlock::Resource { uri: "file://<path>", text: body }`,
 * prepended before the prompt text block.
 */

import { ref, type Ref } from 'vue'

import type { Attachment } from '@ipc'

const pending = ref<Attachment[]>([])

export interface UseAttachments {
  pending: Ref<Attachment[]>
  add: (attachment: Attachment) => void
  remove: (slug: string) => void
  clear: () => void
  has: (slug: string) => boolean
}

export function useAttachments(): UseAttachments {
  return {
    pending,
    add(attachment: Attachment): void {
      if (pending.value.some((a) => a.slug === attachment.slug)) {
        return
      }
      pending.value = [...pending.value, attachment]
    },
    remove(slug: string): void {
      pending.value = pending.value.filter((a) => a.slug !== slug)
    },
    clear(): void {
      pending.value = []
    },
    has(slug: string): boolean {
      return pending.value.some((a) => a.slug === slug)
    }
  }
}
