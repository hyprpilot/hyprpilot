import { computed, reactive, type ComputedRef } from 'vue'

import { ToastTone } from '@components/types'

export interface ToastEntry {
  id: string
  tone: ToastTone
  message: string
  ttlMs: number
  createdAt: number
}

interface ToastsState {
  entries: ToastEntry[]
}

const state = reactive<ToastsState>({ entries: [] })
const MAX_STACK = 3
const DEFAULT_TTL_MS = 4000

let nextId = 0

function allocId(): string {
  nextId += 1
  return `toast-${nextId}`
}

export function pushToast(tone: ToastTone, message: string, ttlMs: number = DEFAULT_TTL_MS): string {
  const id = allocId()
  state.entries.push({ id, tone, message, ttlMs, createdAt: Date.now() })
  while (state.entries.length > MAX_STACK) {
    state.entries.shift()
  }
  window.setTimeout(() => dismissToast(id), ttlMs)
  return id
}

export function dismissToast(id: string): void {
  const idx = state.entries.findIndex((t) => t.id === id)
  if (idx >= 0) {
    state.entries.splice(idx, 1)
  }
}

export function clearToasts(): void {
  state.entries.length = 0
}

export function useToasts(): {
  entries: ComputedRef<ToastEntry[]>
  push: typeof pushToast
  dismiss: typeof dismissToast
} {
  const entries = computed(() => state.entries)
  return { entries, push: pushToast, dismiss: dismissToast }
}
