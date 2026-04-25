/**
 * Skills palette leaf — multi-select catalogue picker. Opens a
 * `PaletteMode.MultiSelect` spec listing every skill from
 * `skills_list`, pre-ticked from the current pending-attachments
 * store (`useAttachments`). On commit the picked slugs round-trip
 * through `skills_get` to snapshot bodies, then replace the pending
 * set with the new selection. Esc closes without committing — pending
 * attachments stay as they were when the palette opened.
 */

import { type PaletteEntry, PaletteMode, usePalette } from '@composables/palette'
import { useAttachments } from '@composables/use-attachments'
import { type Attachment, getSkill, listSkills } from '@ipc'
import { log } from '@lib'

export async function openSkillsLeaf(): Promise<void> {
  const { open } = usePalette()
  const attachments = useAttachments()

  const skills = await listSkills()
  const entries: PaletteEntry[] = skills.map((s) => ({
    id: s.slug,
    name: s.title || s.slug,
    description: s.description
  }))

  const preseedActive: PaletteEntry[] = entries.filter((e) => attachments.has(e.id))

  open({
    mode: PaletteMode.MultiSelect,
    title: 'skills',
    entries,
    preseedActive,
    onCommit(picks) {
      void commitPicks(picks)
    }
  })
}

async function commitPicks(picks: PaletteEntry[]): Promise<void> {
  const attachments = useAttachments()
  const pickedSlugs = new Set(picks.map((p) => p.id))

  // Drop pending entries the user un-ticked.
  for (const slug of attachments.pending.value.map((a) => a.slug)) {
    if (!pickedSlugs.has(slug)) {
      attachments.remove(slug)
    }
  }

  // Snapshot bodies for newly-ticked entries; existing pending entries
  // keep their captured body (re-pick to refresh).
  const fetches = picks.filter((p) => !attachments.has(p.id)).map(async (p) => fetchAndAttach(p.id))
  await Promise.all(fetches)
}

async function fetchAndAttach(slug: string): Promise<void> {
  const body = await getSkill(slug)
  if (!body) {
    log.warn('skills palette: skipped pick — skills_get returned undefined', { slug })

    return
  }
  const attachment: Attachment = {
    slug: body.slug,
    path: body.path,
    body: body.body,
    title: body.title || undefined
  }
  useAttachments().add(attachment)
}
