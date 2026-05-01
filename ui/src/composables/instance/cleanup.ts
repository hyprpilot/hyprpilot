import { type InstanceId } from '../chrome/use-active-instance'
import { resetPhaseSignals } from './use-phase'
import { resetPermissions } from './use-permissions'
import { resetQueue } from './use-queue'
import { resetSessionInfo } from './use-session-info'
import { resetSeq } from './sequence'
import { resetStream } from './use-stream'
import { resetTerminals } from './use-terminals'
import { resetTools } from './use-tools'
import { resetTranscript } from './use-transcript'
import { resetTurns } from './use-turns'

/**
 * Drop every per-instance store entry for `id`. Wired from
 * `use-session-stream` on `InstanceState.Ended` / `Error` so long-running
 * daemons don't accumulate per-instance state across spawn / teardown
 * cycles.
 *
 * New per-instance composables register their `reset*(id)` here so
 * teardown stays a single source of truth.
 */
export function cleanupInstance(id: InstanceId): void {
  resetPermissions(id)
  resetPhaseSignals(id)
  resetQueue(id)
  resetSeq(id)
  resetSessionInfo(id)
  resetStream(id)
  resetTerminals(id)
  resetTools(id)
  resetTranscript(id)
  resetTurns(id)
}
