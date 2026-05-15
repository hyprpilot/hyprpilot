import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import PermissionActions from './PermissionActions.vue'
import type { PermissionOptionView } from '@interfaces/wire/transcript'

/**
 * Pin the daemon-driven default-button selection: when `defaultOptionId`
 * is supplied, ONLY that option renders solid (the visual primary).
 * Other options drop to ghost regardless of their kind. Without
 * `defaultOptionId`, falls back to the legacy `allow_once`-solid rule
 * — kept for compat with older daemon builds that don't ship the
 * field.
 */
describe('PermissionActions', () => {
  const opts: PermissionOptionView[] = [
    {
      optionId: 'allow-once',
      name: 'Allow Once',
      kind: 'allow_once'
    },
    {
      optionId: 'allow-always',
      name: 'Allow Always',
      kind: 'allow_always'
    },
    {
      optionId: 'reject-once',
      name: 'Reject',
      kind: 'reject_once'
    }
  ]

  it('marks the daemon-supplied default option with the data-default attribute', () => {
    const w = mount(PermissionActions, {
      props: { options: opts, defaultOptionId: 'allow-always' }
    })
    const buttons = w.findAll('button')
    const flagged = buttons.filter((b) => b.attributes('data-default') !== undefined)

    expect(flagged).toHaveLength(1)
    expect(flagged[0]!.attributes('aria-label')).toBe('Allow Always')
  })

  it('falls back without a default when no defaultOptionId is supplied', () => {
    const w = mount(PermissionActions, { props: { options: opts } })
    const buttons = w.findAll('button')
    const flagged = buttons.filter((b) => b.attributes('data-default') !== undefined)

    expect(flagged).toHaveLength(0)
  })

  it('emits the raw optionId on click', async() => {
    const w = mount(PermissionActions, {
      props: { options: opts, defaultOptionId: 'allow-once' }
    })

    await w.findAll('button')[2]!.trigger('click')
    const events = w.emitted<['reply', string]>('reply')

    expect(events).toBeDefined()
    expect(events![0]).toEqual(['reject-once'])
  })
})
