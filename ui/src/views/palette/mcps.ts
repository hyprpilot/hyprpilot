/**
 * Multi-select palette leaf for the global `[[mcps]]` catalog. Lists
 * every entry against the active instance's effective set (per-instance
 * override > profile default > all-enabled). On commit, diffs ticked
 * vs baseline and — when the set changed — fires `mcps_set` which
 * triggers a daemon-side restart of the addressed instance. A "ready"
 * toast follows the next `acp:instance-state` transition to `running`
 * for that id; transitions to `error` are reported as warn.
 */

import { type EventCallback, InstanceState, type InstanceStateEventPayload, invoke, listen, TauriCommand, TauriEvent, type UnlistenFn } from '@ipc'

import { ToastTone } from '@components'
import { type PaletteEntry, PaletteMode, usePalette } from '@composables'
import { pushToast } from '@composables'
import { log } from '@lib'

export interface OpenMcpsLeafOptions {
  instanceId: string
  /// Display name for the agent under restart — used in the
  /// "switching MCPs… restarting <agent>" toast. Falls back to
  /// "agent" when the caller doesn't have it on hand.
  agentLabel?: string
}

/**
 * Pure diff helper. Returns true when the post-commit set differs from
 * the baseline (order-insensitive, set semantics). Exposed for testing.
 */
export function mcpsDiffersFromBaseline(baseline: ReadonlySet<string>, ticked: ReadonlySet<string>): boolean {
  if (baseline.size !== ticked.size) {
    return true
  }
  for (const slug of ticked) {
    if (!baseline.has(slug)) {
      return true
    }
  }

  return false
}

async function watchForReady(instanceId: string, agentLabel: string): Promise<void> {
  let unlisten: UnlistenFn | undefined
  let timer: number | undefined
  const cleanup = (): void => {
    if (unlisten) {
      unlisten()
      unlisten = undefined
    }
    if (timer !== undefined) {
      window.clearTimeout(timer)
      timer = undefined
    }
  }
  const cb: EventCallback<InstanceStateEventPayload> = (e) => {
    if (e.payload.instanceId !== instanceId) {
      return
    }
    if (e.payload.state === InstanceState.Running) {
      pushToast(ToastTone.Ok, `${agentLabel}: ready`)
      cleanup()
    } else if (e.payload.state === InstanceState.Error) {
      pushToast(ToastTone.Warn, `${agentLabel}: restart failed`)
      cleanup()
    }
  }
  unlisten = await listen(TauriEvent.AcpInstanceState, cb)
  // Defensive ceiling — drop the listener after 30s even if no
  // matching event arrives so we don't accumulate dead subscribers.
  timer = window.setTimeout(cleanup, 30_000)
}

/**
 * Open the MCP toggle palette. `instanceId` is the active instance the
 * overlay knows about (typically `useActiveInstance().id.value`). The
 * caller is responsible for skipping the call when no active instance
 * is set.
 */
export async function openMcpsLeaf(opts: OpenMcpsLeafOptions): Promise<void> {
  const { instanceId, agentLabel = 'agent' } = opts
  const { open } = usePalette()
  const result = await invoke(TauriCommand.McpsList, { instanceId })
  const items = result.mcps
  const entries: PaletteEntry[] = items.map((m) => ({
    id: m.name,
    name: m.name,
    description: m.command
  }))
  const preseedActive = items.filter((m) => m.enabled).map((m) => ({ id: m.name, name: m.name }))
  const baseline = new Set(preseedActive.map((p) => p.id))

  open({
    mode: PaletteMode.MultiSelect,
    title: 'mcps',
    entries,
    preseedActive,
    onCommit(picks: PaletteEntry[]): void {
      const ticked = new Set(picks.map((p) => p.id))
      if (!mcpsDiffersFromBaseline(baseline, ticked)) {
        return
      }
      const enabled = [...ticked]
      pushToast(ToastTone.Info, `switching MCPs… restarting ${agentLabel}`)
      void invoke(TauriCommand.McpsSet, { instanceId, enabled })
        .then(() => watchForReady(instanceId, agentLabel))
        .catch((err) => {
          log.warn('mcps_set failed', { instanceId }, err)
          pushToast(ToastTone.Err, `mcps: failed to restart ${agentLabel}`)
        })
    }
  })
}
