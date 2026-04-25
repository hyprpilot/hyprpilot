/**
 * Slash-commands palette leaf (K-267). Lists the agent's
 * `available_commands` for the active instance via the
 * `commands_list` Tauri command; selecting a row inserts
 * `/<name> ` at the composer caret and closes the palette
 * without submitting (the user finalises the prompt before
 * sending).
 *
 * The backing cache lands in K-251 — until then the wire
 * surfaces `-32603 not implemented`; the catch arm logs +
 * leaves the palette in its empty state. Same shape applies
 * when the active instance hasn't spawned yet (no
 * `instanceId`): we open a palette with an explanatory
 * empty-state row rather than swallowing the call.
 */

import { type PaletteEntry, PaletteMode, usePalette } from '@composables/palette'
import { useActiveInstance } from '@composables/use-active-instance'
import { useComposer } from '@composables/use-composer'
import { invoke, TauriCommand, type SlashCommand } from '@ipc'
import { log } from '@lib'

const LOADING_ID = '__loading__'
const NO_INSTANCE_ID = '__no-instance__'
const EMPTY_ID = '__empty__'
const ERROR_ID = '__error__'

function placeholder(id: string, name: string, description?: string): PaletteEntry {
  return { id, name, description }
}

export async function openCommandsLeaf(): Promise<void> {
  const { open, close } = usePalette()
  const { id: activeInstanceId } = useActiveInstance()
  const composer = useComposer()

  const instanceId = activeInstanceId.value
  if (!instanceId) {
    open({
      mode: PaletteMode.Select,
      title: 'commands',
      entries: [placeholder(NO_INSTANCE_ID, 'no active instance', 'send a turn first to spawn one')],
      onCommit: () => {}
    })

    return
  }

  open({
    mode: PaletteMode.Select,
    title: 'commands',
    entries: [placeholder(LOADING_ID, 'loading…')],
    onCommit: () => {}
  })

  let commands: SlashCommand[]
  try {
    const r = await invoke(TauriCommand.CommandsList, { instanceId })
    commands = r.commands
  } catch (err) {
    log.warn('commands_list failed', { err: String(err) })
    close()
    open({
      mode: PaletteMode.Select,
      title: 'commands',
      entries: [placeholder(ERROR_ID, 'commands unavailable', 'agent has not advertised any')],
      onCommit: () => {}
    })

    return
  }

  // Replace the loading placeholder with the resolved list. The
  // active palette spec is captured in `usePalette().stack`; pop +
  // push lands the new spec on the same level so the user doesn't
  // see a flash back to the parent root palette.
  close()

  if (commands.length === 0) {
    open({
      mode: PaletteMode.Select,
      title: 'commands',
      entries: [placeholder(EMPTY_ID, 'no commands', 'agent has not advertised any')],
      onCommit: () => {}
    })

    return
  }

  const entries: PaletteEntry[] = commands.map((c) => ({
    id: c.name,
    name: `/${c.name}`,
    description: c.description
  }))

  open({
    mode: PaletteMode.Select,
    title: 'commands',
    entries,
    onCommit(picks) {
      const pick = picks[0]
      if (!pick) {
        return
      }
      // Filter the synthetic placeholder ids — they're inert rows
      // that the user can highlight + Enter on, but they should
      // never insert their own id as a slash-command.
      if (pick.id === LOADING_ID || pick.id === EMPTY_ID || pick.id === ERROR_ID || pick.id === NO_INSTANCE_ID) {
        return
      }
      composer.insertAtCaret(`/${pick.id} `)
      composer.focus()
    }
  })
}
