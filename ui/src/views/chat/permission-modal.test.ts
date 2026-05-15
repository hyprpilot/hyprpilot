import { faTerminal } from '@fortawesome/free-solid-svg-icons'
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import PermissionModal from './PermissionModal.vue'
import { PermissionUi } from '@components'
import { ToolKind, ToolState } from '@constants/ui'
import type { PermissionView } from '@interfaces/ui'

/**
 * Pin the reject-feedback gate on the modal:
 * - Click an allow option with text in the textarea → feedback is
 *   dropped (allow path doesn't dispatch a daemon follow-up turn).
 * - Click reject with empty / whitespace-only feedback → still
 *   emits without a feedback arg (preserves the legacy single-arg
 *   shape).
 * - Click reject with non-empty trimmed feedback → emits the
 *   trimmed string as the second arg.
 *
 * The daemon's permissions/respond handler already gates feedback
 * to reject-shaped picks, so the modal-side gate is a defensive
 * mirror — keeps `acp:permission-request` event traffic free of
 * accidental allow-with-feedback shapes that the daemon would
 * silently drop anyway.
 */
function buildView(): PermissionView {
  return {
    request: {
      requestId: 'r-1',
      instanceId: 'i-1',
      sessionId: 's-1',
      toolName: 'Bash'
    },
    call: {
      id: 'tc-1',
      name: 'Bash',
      title: 'Bash',
      icon: faTerminal,
      kind: ToolKind.Other,
      state: ToolState.Pending,
      stats: [],
      fields: [],
      permissionUi: PermissionUi.Modal
    },
    options: [
      {
        optionId: 'allow-once',
        name: 'Allow',
        kind: 'allow_once'
      },
      {
        optionId: 'reject-once',
        name: 'Reject',
        kind: 'reject_once'
      }
    ],
    defaultOptionId: 'allow-once'
  }
}

describe('PermissionModal', () => {
  it('drops feedback on the allow path even when the textarea is populated', async() => {
    const w = mount(PermissionModal, { props: { view: buildView() } })

    await w.find('textarea').setValue('this is a reason but I clicked allow')
    const allow = w.findAll('button').find((b) => b.attributes('aria-label') === 'Allow')!

    await allow.trigger('click')
    const events = w.emitted<['reply', string, string | undefined]>('reply')

    expect(events).toBeDefined()
    // Allow path: single-arg call shape — no feedback piggybacks.
    expect(events![0]).toEqual(['allow-once'])
  })

  it('drops feedback on reject when the textarea is whitespace-only', async() => {
    const w = mount(PermissionModal, { props: { view: buildView() } })

    await w.find('textarea').setValue('   \n  ')
    const reject = w.findAll('button').find((b) => b.attributes('aria-label') === 'Reject')!

    await reject.trigger('click')
    const events = w.emitted<['reply', string, string | undefined]>('reply')

    expect(events).toBeDefined()
    expect(events![0]).toEqual(['reject-once'])
  })

  it('emits trimmed feedback on reject with non-empty input', async() => {
    const w = mount(PermissionModal, { props: { view: buildView() } })

    await w.find('textarea').setValue('  path looks unsafe, try /tmp instead  ')
    const reject = w.findAll('button').find((b) => b.attributes('aria-label') === 'Reject')!

    await reject.trigger('click')
    const events = w.emitted<['reply', string, string | undefined]>('reply')

    expect(events).toBeDefined()
    expect(events![0]).toEqual(['reject-once', 'path looks unsafe, try /tmp instead'])
  })
})
