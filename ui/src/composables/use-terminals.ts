import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { useActiveInstance, type InstanceId } from './use-active-instance'

export interface TerminalStream {
  toolCallId: string
  sessionId: string
  command?: string
  cwd?: string
  stdout: string
  running: boolean
  exitCode?: number
  createdAt: number
  updatedAt: number
}

export interface TerminalsState {
  streams: Record<string, TerminalStream>
}

const states = reactive(new Map<InstanceId, TerminalsState>())

function slotFor(id: InstanceId): TerminalsState {
  let slot = states.get(id)
  if (!slot) {
    slot = { streams: {} }
    states.set(id, slot)
  }

  return slot
}

export interface TerminalChunk {
  toolCallId: string
  sessionId: string
  command?: string
  cwd?: string
  stdout?: string
  running?: boolean
  exitCode?: number
}

export function pushTerminalChunk(id: InstanceId, chunk: TerminalChunk): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const existing = slot.streams[chunk.toolCallId]
  if (existing) {
    existing.updatedAt = seq
    if (chunk.command !== undefined) {
      existing.command = chunk.command
    }
    if (chunk.cwd !== undefined) {
      existing.cwd = chunk.cwd
    }
    if (typeof chunk.stdout === 'string') {
      existing.stdout += chunk.stdout
    }
    if (chunk.running !== undefined) {
      existing.running = chunk.running
    }
    if (chunk.exitCode !== undefined) {
      existing.exitCode = chunk.exitCode
    }
    return
  }
  slot.streams[chunk.toolCallId] = {
    toolCallId: chunk.toolCallId,
    sessionId: chunk.sessionId,
    command: chunk.command,
    cwd: chunk.cwd,
    stdout: chunk.stdout ?? '',
    running: chunk.running ?? true,
    exitCode: chunk.exitCode,
    createdAt: seq,
    updatedAt: seq
  }
}

export function resetTerminals(id: InstanceId): void {
  states.delete(id)
}

export function useTerminals(instanceId?: InstanceId): { streams: ComputedRef<Record<string, TerminalStream>> } {
  const { id: activeId } = useActiveInstance()
  const streams = computed<Record<string, TerminalStream>>(() => {
    const resolved = instanceId ?? activeId.value
    if (!resolved) {
      return {}
    }

    return states.get(resolved)?.streams ?? {}
  })

  return { streams }
}
