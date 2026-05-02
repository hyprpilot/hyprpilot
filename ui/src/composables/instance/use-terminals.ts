/**
 * Live terminal store (K-257). The Rust runtime pushes one
 * `acp:terminal` event per stdout / stderr chunk + once on exit; this
 * module accumulates those into a per-`terminalId` view the
 * `ChatTerminalCard` reads. Output is capped at 2000 lines per
 * terminal — older lines drop in arrival order so a runaway child
 * can't pin the webview's memory while we wait for the user to stop
 * watching.
 *
 * State is keyed `(instanceId, terminalId)` so concurrent terminals
 * across instances stay isolated. The composable exposes
 * `byId(terminalId)` for inline-card binding + `all` for future
 * full-screen terminal listings.
 */
import { computed, reactive, type ComputedRef } from 'vue'

import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'

/** Cap matches the size of a couple of full editor scrollbacks — pilot.py used a similar threshold. */
const MAX_LINES = 2000

export interface TerminalEntry {
  id: string
  command?: string
  cwd?: string
  output: string
  truncated: boolean
  exitCode?: number
  signal?: string
  running: boolean
  createdAt: number
}

interface InstanceSlot {
  byId: Map<string, TerminalEntry>
  order: string[]
  seq: number
}

const states = reactive(new Map<InstanceId, InstanceSlot>())

function slotFor(id: InstanceId): InstanceSlot {
  let slot = states.get(id)

  if (!slot) {
    slot = {
      byId: new Map(),
      order: [],
      seq: 0
    }
    states.set(id, slot)
  }

  return slot
}

function entryFor(slot: InstanceSlot, terminalId: string): TerminalEntry {
  let entry = slot.byId.get(terminalId)

  if (!entry) {
    slot.seq += 1
    entry = {
      id: terminalId,
      output: '',
      truncated: false,
      running: true,
      createdAt: slot.seq
    }
    slot.byId.set(terminalId, entry)
    slot.order.push(terminalId)
  }

  return entry
}

/** Append `data` to `entry.output`, dropping oldest lines once we exceed `MAX_LINES`. */
function appendCapped(entry: TerminalEntry, data: string): void {
  entry.output += data
  const lines = entry.output.split('\n')

  if (lines.length > MAX_LINES + 1) {
    // +1 because trailing '' from a newline-terminated buffer counts as a "line"
    const drop = lines.length - (MAX_LINES + 1)

    entry.output = lines.slice(drop).join('\n')
    entry.truncated = true
  }
}

export interface TerminalChunkInput {
  terminalId: string
  data: string
  command?: string
  cwd?: string
}

export interface TerminalExitInput {
  terminalId: string
  exitCode?: number
  signal?: string
}

/** Push a stdout / stderr delta into the per-instance store. Hydrates `command` / `cwd` if supplied. */
export function pushTerminalChunk(id: InstanceId, chunk: TerminalChunkInput): void {
  const slot = slotFor(id)
  const entry = entryFor(slot, chunk.terminalId)

  if (chunk.command !== undefined) {
    entry.command = chunk.command
  }

  if (chunk.cwd !== undefined) {
    entry.cwd = chunk.cwd
  }

  if (chunk.data) {
    appendCapped(entry, chunk.data)
  }
}

/** Resolve the running flag and the `(exitCode, signal)` pair. Subsequent chunks are no-ops. */
export function pushTerminalExit(id: InstanceId, exit: TerminalExitInput): void {
  const slot = slotFor(id)
  const entry = entryFor(slot, exit.terminalId)

  entry.running = false

  if (exit.exitCode !== undefined) {
    entry.exitCode = exit.exitCode
  }

  if (exit.signal !== undefined) {
    entry.signal = exit.signal
  }
}

export function resetTerminals(id: InstanceId): void {
  states.delete(id)
}

export interface UseTerminals {
  byId: (terminalId: string) => ComputedRef<TerminalEntry | undefined>
  all: ComputedRef<TerminalEntry[]>
}

export function useTerminals(instanceId?: InstanceId): UseTerminals {
  const { id: activeId } = useActiveInstance()
  const resolved = (): InstanceId | undefined => instanceId ?? activeId.value

  return {
    byId(terminalId: string) {
      return computed(() => {
        const id = resolved()

        if (!id) {
          return undefined
        }

        return states.get(id)?.byId.get(terminalId)
      })
    },
    all: computed<TerminalEntry[]>(() => {
      const id = resolved()

      if (!id) {
        return []
      }
      const slot = states.get(id)

      if (!slot) {
        return []
      }

      return slot.order.map((tid) => slot.byId.get(tid)).filter((e): e is TerminalEntry => e !== undefined)
    })
  }
}
