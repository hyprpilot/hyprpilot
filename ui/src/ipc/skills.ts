/**
 * Wrappers around the `skills_list` + `skills_get` Tauri commands
 * (K-268, K-269). The skills palette calls `listSkills()` on open,
 * then `getSkill(slug)` on commit-tick to snapshot each picked skill's
 * body into an `Attachment`. Errors fall through to `undefined` so a
 * stale slug (deleted, renamed, registry out of sync) surfaces as a
 * no-op pill rather than crashing the palette.
 */

import { log } from '@lib'

import { invoke } from './bridge'
import { TauriCommand } from './commands'
import type { SkillBody, SkillSummary } from './types'

export async function listSkills(): Promise<SkillSummary[]> {
  try {
    const r = await invoke(TauriCommand.SkillsList)

    return r.skills
  } catch (err) {
    log.debug('skills_list invoke failed', { err: String(err) })

    return []
  }
}

export async function getSkill(slug: string): Promise<SkillBody | undefined> {
  try {
    return await invoke(TauriCommand.SkillsGet, { slug })
  } catch (err) {
    log.debug('skills_get invoke failed', { slug, err: String(err) })

    return undefined
  }
}
