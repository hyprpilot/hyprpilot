/**
 * Per-instance git-status driver. Watches the active instance's cwd
 * and pulls a fresh snapshot (`get_git_status`) on every change.
 * The result lands on the instance's session-info slot via
 * `setInstanceGitStatus`, where the header pill reads it.
 *
 * One-shot per cwd change — git status doesn't tick like a clock,
 * and a long-poll loop would burn a libgit2 walk on every interval
 * regardless of activity. If captains want richer (post-commit /
 * post-fetch) refreshes later we add a manual refresh keybind, not
 * a timer.
 *
 * Soft-fails on any IPC / libgit2 error: the pill simply doesn't
 * paint. Unobserved errors land in the dev console as warnings.
 */
import { watch } from 'vue'

import { peekSessionInfo, setInstanceGitStatus, useSessionInfo } from './use-session-info'
import { onTurnEnded } from './use-turns'
import { useActiveInstance } from '../chrome/use-active-instance'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

let started = false

async function refresh(instanceId: string, cwd: string): Promise<void> {
  try {
    const status = await invoke(TauriCommand.GetGitStatus, { path: cwd })

    setInstanceGitStatus(instanceId, status ?? undefined)
  } catch(err) {
    log.warn('get_git_status failed', { err: String(err), cwd })
  }
}

export function startGitStatus(): void {
  if (started) {
    return
  }
  started = true

  const { id: activeId } = useActiveInstance()
  const { info } = useSessionInfo()

  // (1) cwd-change driver: any time the active instance's cwd flips
  // (instance switch / cwd palette commit / session restore), pull a
  // fresh snapshot.
  watch(
    () => ({ id: activeId.value, cwd: info.value.cwd }),
    async(next) => {
      if (!next.id || !next.cwd) {
        return
      }
      const instanceId = next.id

      await refresh(instanceId, next.cwd)

      // Active instance can flip during the await; the refresh
      // would already have written into the right slot, but if the
      // captain switched instances mid-call we don't need to do
      // anything else.
    },
    { immediate: true }
  )

  // (2) turn-ended driver: agent turns commit / pull / branch-switch
  // through tool calls, so the captain's git state is most likely
  // stale right when a turn finishes. Re-snapshot the impacted
  // instance's cwd on every TurnEnded regardless of stop reason.
  onTurnEnded((instanceId) => {
    const slot = peekSessionInfo(instanceId)

    if (!slot?.cwd) {
      return
    }
    void refresh(instanceId, slot.cwd)
  })
}
