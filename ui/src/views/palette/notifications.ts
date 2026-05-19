/**
 * Notifications palette leaf. Lists every instance flagged as
 * needing attention; `Enter` focuses one (the daemon then clears the
 * entry via the focus event), `Ctrl+D` dismisses just that row.
 *
 * The first row, when there are pending entries, is a synthetic
 * "dismiss all" action — captain hits Enter on it to drop every
 * pending entry in one shot via `notifications_clear_all`.
 *
 * The leaf reads the live mirror (the daemon broadcasts every state
 * transition on `acp:notifications-changed`; the composable mirrors
 * it). No round-trip to open the leaf — the snapshot is already in
 * memory.
 */

import { ToastTone } from '@components'
import { type PaletteEntry, PaletteMode, type PaletteSpec, pushToast, usePalette, usePhase, useSessionInfo, useNotifications, type InstanceId } from '@composables'
import { invoke, NotificationReason, TauriCommand, type NotificationEntry } from '@ipc'
import { log } from '@lib'

const DISMISS_ALL_ID = '__notifications_dismiss_all__'

function reasonLabel(reason: NotificationReason): string {
  switch (reason) {
    case NotificationReason.TurnEnded:
      return 'turn ended'
    case NotificationReason.PermissionRequested:
      return 'permission'
    case NotificationReason.InstanceError:
      return 'error'
  }
}

function ageLabel(sinceMs: number): string {
  const seconds = Math.max(0, Math.floor((Date.now() - sinceMs) / 1000))

  if (seconds < 60) {
    return `${seconds}s ago`
  }
  const minutes = Math.floor(seconds / 60)

  if (minutes < 60) {
    return `${minutes}m ago`
  }
  const hours = Math.floor(minutes / 60)

  return `${hours}h ago`
}

function rowFor(entry: NotificationEntry): PaletteEntry {
  // Headline reads the same axes as the instances leaf so the captain
  // stays in one vocabulary across surfaces — captain-set name first,
  // session title next, then profile id, then the bare instance id
  // slug (always present).
  const { info } = useSessionInfo(entry.instanceId)
  const { phase } = usePhase(entry.instanceId)
  const sess = info.value
  const headline = sess.name ?? sess.title ?? sess.profileId ?? entry.instanceId

  const reasons = entry.reasons.map(reasonLabel).join(' · ')
  const meta: string[] = [reasons, ageLabel(entry.since)]

  if (sess.agent) {
    meta.push(sess.agent)
  }
  meta.push(phase.value)

  return {
    id: entry.instanceId,
    name: headline,
    description: meta.join(' · '),
    kind: entry.instanceId.slice(0, 8)
  }
}

async function focusInstance(id: InstanceId): Promise<void> {
  try {
    await invoke(TauriCommand.InstancesFocus, { instanceId: id })
  } catch(err) {
    log.error('invoke failed', { command: TauriCommand.InstancesFocus, id }, err)
    pushToast(ToastTone.Err, `instances focus failed: ${String(err)}`)
  }
}

async function dismissOne(id: InstanceId): Promise<void> {
  try {
    await invoke(TauriCommand.NotificationsClear, { instanceId: id })
  } catch(err) {
    log.error('invoke failed', { command: TauriCommand.NotificationsClear, id }, err)
    pushToast(ToastTone.Err, `notifications dismiss failed: ${String(err)}`)
  }
}

async function dismissAll(): Promise<void> {
  try {
    await invoke(TauriCommand.NotificationsClearAll)
  } catch(err) {
    log.error('invoke failed', { command: TauriCommand.NotificationsClearAll }, err)
    pushToast(ToastTone.Err, `notifications dismiss-all failed: ${String(err)}`)
  }
}

function entriesFor(items: NotificationEntry[]): PaletteEntry[] {
  if (items.length === 0) {
    return [
      {
        id: 'notifications-empty',
        name: 'nothing needs attention.'
      }
    ]
  }

  return [
    {
      id: DISMISS_ALL_ID,
      name: `dismiss all (${items.length})`,
      description: 'clear every pending notification'
    },
    ...items.map(rowFor)
  ]
}

export function openNotificationsLeaf(): void {
  const palette = usePalette()
  const { items } = useNotifications()

  const spec = {
    mode: PaletteMode.Select,
    title: 'notifications',
    entries: entriesFor(items.value),
    onCommit(picks: PaletteEntry[]) {
      const pick = picks[0]

      if (!pick || pick.id === 'notifications-empty') {
        return
      }

      if (pick.id === DISMISS_ALL_ID) {
        void dismissAll()

        return
      }
      void focusInstance(pick.id)
    },
    async onDelete(entry: PaletteEntry, update: (entries: PaletteEntry[]) => void) {
      if (entry.id === 'notifications-empty' || entry.id === DISMISS_ALL_ID) {
        return
      }
      await dismissOne(entry.id)
      // The daemon broadcast already trimmed the mirror — re-derive
      // entries from the now-current list so the captain sees the
      // shorter list without re-opening the palette.
      update(entriesFor(items.value))
    }
  } satisfies PaletteSpec

  palette.open(spec)
}
