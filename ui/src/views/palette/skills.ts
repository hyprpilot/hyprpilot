/**
 * skills palette leaf — single action for now: re-scan every
 * configured skills root. The fs watcher was dropped because edit
 * noise from editors / git ops burnt through its debouncer faster
 * than skills changed; this leaf is the explicit captain-driven
 * trigger.
 */

import { ToastTone } from '@components'
import { PaletteMode, usePalette, useToasts } from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

const RELOAD_ROW_ID = 'skills-reload'

async function commitReload(): Promise<void> {
  const toasts = useToasts()

  try {
    const { count } = await invoke(TauriCommand.SkillsReload)

    toasts.push(ToastTone.Ok, `skills reloaded — ${count} loaded`)
  } catch(err) {
    log.warn('palette-skills: skills_reload failed', { err: String(err) })
    toasts.push(ToastTone.Err, `skills reload failed: ${String(err)}`)
  }
}

export function openSkillsLeaf(): void {
  const { open } = usePalette()

  open({
    mode: PaletteMode.Select,
    title: 'skills',
    entries: [
      {
        id: RELOAD_ROW_ID,
        name: 'reload.'
      }
    ],
    onCommit(picks) {
      const pick = picks[0]

      if (pick?.id === RELOAD_ROW_ID) {
        void commitReload()
      }
    }
  })
}
