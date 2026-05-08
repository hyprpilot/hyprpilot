/**
 * TanStack Query wrapper around the daemon's
 * `instance_snapshot_terminals` Tauri command. Fetches the full
 * per-`terminalId` map for one instance off the per-instance write-
 * through mirror. Small enough to ship whole today; revisit if
 * sessions accumulate dozens of long-running terminals.
 *
 * Drives the `<TerminalDrawer>` collapse-driven unmount
 * (Phase C1) — expanding the drawer triggers this fetch; collapsing
 * the drawer evicts the heavy `stdout` / `stderr` payloads from
 * memory via TanStack's `gcTime`. Disabled when no instance is in
 * focus.
 */

import { useQuery } from '@tanstack/vue-query'
import { computed, type ComputedRef } from 'vue'

import { type InstanceId } from '../chrome/use-active-instance'
import { invoke, TauriCommand, type TerminalsSnapshot } from '@ipc'

export type UseInstanceTerminalsQueryReturn = ReturnType<typeof useQuery<TerminalsSnapshot, Error, TerminalsSnapshot, unknown[]>>

export function useInstanceTerminalsQuery(instanceId: ComputedRef<InstanceId | undefined>): UseInstanceTerminalsQueryReturn {
  return useQuery({
    queryKey: computed(() => ['snapshot-terminals', instanceId.value]),
    enabled: computed(() => instanceId.value !== undefined),
    queryFn: async() => {
      const id = instanceId.value

      if (id === undefined) {
        throw new Error('useInstanceTerminalsQuery: instanceId is undefined')
      }

      return invoke(TauriCommand.InstanceSnapshotTerminals, { instanceId: id })
    }
  })
}
