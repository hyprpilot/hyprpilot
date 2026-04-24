/**
 * Wrapper around the `skills_get` Tauri command (K-268). The skills
 * palette calls `getSkill(slug)` on tick, snapshots the body into an
 * `Attachment`, and pushes it through `useAttachments`. Errors fall
 * through to `undefined` so a stale slug (deleted, renamed, registry
 * out of sync) surfaces as a no-op pill rather than crashing the
 * palette.
 */

import { log } from '@lib'

import { invoke } from './bridge'
import { TauriCommand } from './commands'
import type { SkillBody } from './types'

export async function getSkill(slug: string): Promise<SkillBody | undefined> {
  try {
    return await invoke(TauriCommand.SkillsGet, { slug })
  } catch (err) {
    log.debug('skills_get invoke failed', { slug, err: String(err) })

    return undefined
  }
}
