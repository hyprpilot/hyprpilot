/**
 * Pin the composable's wire surface: `applyBootNotifications` seeds
 * the singleton, `count` is reactive off `items`, `dismiss` /
 * `dismissAll` route to the matching Tauri commands.
 *
 * The event subscription path lives in `ensureSubscribed` and runs as
 * a side effect of the first `useNotifications()` call; we don't pin
 * it here (jsdom + Tauri's `listen` mock would require a wider
 * stub than is justified for this slice).
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetNotificationsForTests, applyBootNotifications, useNotifications } from './use-notifications'
import { NotificationReason, type NotificationEntry } from '@ipc'

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn<(event: string, cb: (e: { payload: unknown }) => void) => Promise<() => void>>()
}))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args),
  listen: (event: string, cb: (e: { payload: unknown }) => void) => listenMock(event, cb)
}))

beforeEach(() => {
  invokeMock.mockReset()
  invokeMock.mockResolvedValue({ cleared: true })
  listenMock.mockReset()
  listenMock.mockResolvedValue(() => {})
  __resetNotificationsForTests()
})

function entry(instanceId: string, reasons: NotificationReason[] = [NotificationReason.TurnEnded]): NotificationEntry {
  return {
    instanceId, reasons, since: 0
  }
}

describe('useNotifications', () => {
  it('applyBootNotifications seeds the singleton and exposes a reactive count', () => {
    applyBootNotifications({ items: [entry('a'), entry('b')] })

    const { items, count } = useNotifications()

    expect(items.value).toHaveLength(2)
    expect(count.value).toBe(2)
  })

  it('applyBootNotifications no-ops on undefined', () => {
    applyBootNotifications(undefined)

    const { items } = useNotifications()

    expect(items.value).toEqual([])
  })

  it('dismiss routes to NotificationsClear with the instanceId', async() => {
    const { dismiss } = useNotifications()

    await dismiss('inst-1')

    expect(invokeMock).toHaveBeenCalledWith('notifications_clear', { instanceId: 'inst-1' })
  })

  it('dismissAll routes to NotificationsClearAll', async() => {
    const { dismissAll } = useNotifications()

    await dismissAll()

    expect(invokeMock).toHaveBeenCalledWith('notifications_clear_all', undefined)
  })

  it('forInstance reactively tracks the entry by id', () => {
    applyBootNotifications({
      items: [entry('a'), entry('b', [NotificationReason.PermissionRequested])]
    })

    const { forInstance } = useNotifications()
    const aRow = forInstance('a')
    const bRow = forInstance('b')
    const missing = forInstance('zzz')

    expect(aRow.value?.instanceId).toBe('a')
    expect(bRow.value?.reasons).toEqual([NotificationReason.PermissionRequested])
    expect(missing.value).toBeUndefined()

    applyBootNotifications({ items: [entry('a')] })
    expect(bRow.value).toBeUndefined()
  })
})
