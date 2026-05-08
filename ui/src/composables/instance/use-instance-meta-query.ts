/**
 * TanStack Query wrapper around the daemon's
 * `instance_snapshot_meta` Tauri command. Fetches the small
 * "header / chrome" view (mode, model, advertised modes/models, cwd,
 * mcps_count, profile id, current turn marker, pending permissions,
 * usage tally) for one instance off the per-instance write-through
 * mirror.
 *
 * Used during brim-sync hydration on `acp:instances-focused` /
 * `acp:instances-changed` (Phase C2) so the chat header / pickers
 * settle without waiting for the next live event. Disabled when no
 * instance is in focus.
 */

import { useQuery } from '@tanstack/vue-query'
import { computed, type ComputedRef } from 'vue'

import { type InstanceId } from '../chrome/use-active-instance'
import { invoke, TauriCommand, type MetaSnapshot } from '@ipc'

export type UseInstanceMetaQueryReturn = ReturnType<typeof useQuery<MetaSnapshot, Error, MetaSnapshot, unknown[]>>

export function useInstanceMetaQuery(instanceId: ComputedRef<InstanceId | undefined>): UseInstanceMetaQueryReturn {
  return useQuery({
    queryKey: computed(() => ['snapshot-meta', instanceId.value]),
    enabled: computed(() => instanceId.value !== undefined),
    queryFn: async() => {
      const id = instanceId.value

      if (id === undefined) {
        // Guard: `enabled` should prevent this branch, but TS can't
        // see through the computed ref so we keep the runtime check.
        throw new Error('useInstanceMetaQuery: instanceId is undefined')
      }

      return invoke(TauriCommand.InstanceSnapshotMeta, { instanceId: id })
    }
  })
}
